//! The MariaDB helper's save logic, without a server: positions, manifests and blobs.
#![cfg(feature = "mariadb")]
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use arena_part::helpers::mariadb::{
    chain_error, check_same_server, dump_position, names_db, needed_positions, percent_encode,
    purge_target, query_misplaced, restore_error, saves_in, span, span_bytes, to_reroot,
    used_roots, BinlogFile, Blobs, Dumped, Manifest, Misplaced, Position, Purged, Segment,
    ServerRef, Standing, Tracked, TrackedChange,
};
use arena_part::helpers::temp_dir::TempDir;

fn pos(file: &str, pos: u64) -> Position {
    Position::new(file, pos)
}

fn files() -> Vec<BinlogFile> {
    [
        ("binlog.000002", 1000),
        ("binlog.000003", 500),
        ("binlog.000010", 300),
    ]
    .into_iter()
    .map(|(name, size)| BinlogFile::new(name, size))
    .collect()
}

fn server() -> ServerRef {
    ServerRef {
        container: "arena-mariadb".into(),
        port: 3307,
        id: "5f0c7a52-0000-4000-8000-000000000001".into(),
    }
}

fn seg(db: &str, from: Position, to: Position) -> Segment {
    Segment {
        db: db.into(),
        from,
        to,
    }
}

fn scratch() -> TempDir {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    TempDir::new_in(&root, "mariadb_blobs").unwrap()
}

#[test]
fn positions_order_by_file_number_then_offset() {
    assert!(pos("binlog.000009", 900) < pos("binlog.000010", 4));
    assert!(pos("binlog.000010", 4) < pos("binlog.000010", 5));
    assert!(pos("binlog.000100", 4) > pos("binlog.000099", 4000));
    assert_eq!(pos("binlog.000003", 7), pos("binlog.000003", 7));
}

#[test]
fn span_lists_files_between_positions() {
    let f = files();
    let names = |v: Vec<&BinlogFile>| v.iter().map(|f| f.name.clone()).collect::<Vec<_>>();
    let s = span(&f, &pos("binlog.000002", 10), &pos("binlog.000010", 20)).unwrap();
    assert_eq!(
        names(s),
        ["binlog.000002", "binlog.000003", "binlog.000010"]
    );
    let s = span(&f, &pos("binlog.000003", 10), &pos("binlog.000003", 20)).unwrap();
    assert_eq!(names(s), ["binlog.000003"]);
    // the first file was purged
    let e = span(&f, &pos("binlog.000001", 10), &pos("binlog.000003", 20)).unwrap_err();
    assert!(e.to_string().contains("binlog.000001"), "{e}");
}

#[test]
fn span_bytes_counts_partial_ends_and_whole_middles() {
    let f = files();
    assert_eq!(
        span_bytes(&f, &pos("binlog.000003", 100), &pos("binlog.000003", 400)),
        300
    );
    assert_eq!(
        span_bytes(&f, &pos("binlog.000002", 900), &pos("binlog.000010", 50)),
        100 + 500 + 50
    );
}

#[test]
fn manifest_appends_segments_and_skips_empty_ones() {
    let base = Manifest::empty(server());
    let a = base.then(seg(
        "mi_a",
        pos("binlog.000002", 10),
        pos("binlog.000002", 90),
    ));
    assert_eq!(a.segments.len(), 1);
    assert!(base.segments.is_empty(), "the base manifest is unchanged");
    let same = a.then(seg(
        "mi_b",
        pos("binlog.000003", 5),
        pos("binlog.000003", 5),
    ));
    assert_eq!(same, a, "an empty segment adds nothing");
    let b = a.then(seg(
        "mi_b",
        pos("binlog.000003", 5),
        pos("binlog.000010", 7),
    ));
    assert_eq!(b.segments.len(), 2);
    assert_eq!(b.segments[1].db, "mi_b");
    assert_eq!(b.oldest(), Some(&pos("binlog.000002", 10)));
    assert_eq!(Manifest::empty(server()).oldest(), None);
}

#[test]
fn manifest_binlog_bytes_and_dump_threshold() {
    let f = files();
    let m = Manifest::empty(server())
        .then(seg(
            "mi_a",
            pos("binlog.000002", 0),
            pos("binlog.000002", 600),
        ))
        .then(seg(
            "mi_b",
            pos("binlog.000003", 100),
            pos("binlog.000003", 300),
        ));
    assert_eq!(m.binlog_bytes(&f), 800);
    assert!(!m.needs_dump(&f, 800));
    assert!(m.needs_dump(&f, 799));
}

#[test]
fn manifest_reads_back_what_it_wrote() {
    let dir = scratch();
    let m = Manifest::rooted(server(), "ab".repeat(32)).then(seg(
        "mi_a",
        pos("binlog.000002", 1),
        pos("binlog.000003", 2),
    ));
    let bytes = m.write(dir.path()).unwrap();
    assert!(bytes > 0);
    let names: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        names,
        ["manifest.json"],
        "written by rename, no temp file left"
    );
    assert_eq!(Manifest::read(dir.path()).unwrap(), m);
    let missing = Manifest::read(&dir.path().join("missing")).unwrap_err();
    assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn purge_target_is_the_oldest_needed_file_or_the_current_one() {
    let f = files();
    let current = pos("binlog.000010", 300);
    let needed = [pos("binlog.000010", 50), pos("binlog.000003", 999)];
    assert_eq!(purge_target(&needed, &current, &f), Some("binlog.000003"));
    assert_eq!(purge_target(&[], &current, &f), Some("binlog.000010"));
}

#[test]
fn purge_target_is_clamped_to_files_on_the_server() {
    let f = files();
    let current = pos("binlog.000010", 300);
    // already purged past it: nothing to do
    assert_eq!(purge_target(&[pos("binlog.000001", 4)], &current, &f), None);
    // the first file on the server: nothing before it
    assert_eq!(purge_target(&[pos("binlog.000002", 4)], &current, &f), None);
    // a needed file that is missing in the middle: the newest one before it
    assert_eq!(
        purge_target(&[pos("binlog.000007", 4)], &current, &f),
        Some("binlog.000003")
    );
}

#[test]
fn dump_head_notes_the_position() {
    let head = "-- Position\n--\n\n-- CHANGE MASTER TO MASTER_LOG_FILE='binlog.000002', \
                MASTER_LOG_POS=841;\n\nCREATE TABLE t (id int);";
    assert_eq!(dump_position(head), Some(pos("binlog.000002", 841)));
    assert_eq!(dump_position("CREATE TABLE t (id int);"), None);
}

#[test]
fn passwords_are_percent_encoded_for_urls() {
    assert_eq!(percent_encode("ab-C.9_~"), "ab-C.9_~");
    assert_eq!(percent_encode("p@ss:w/rd%"), "p%40ss%3Aw%2Frd%25");
}

#[test]
fn blobs_are_content_addressed_and_removed_when_unreferenced() {
    let dir = scratch();
    let blobs = Blobs::new(dir.path().join(".blobs"));
    let a = blobs.write_from(&b"CREATE TABLE t(id int);"[..]).unwrap();
    let a2 = blobs.write_from(&b"CREATE TABLE t(id int);"[..]).unwrap();
    let b = blobs.write_from(&b"CREATE TABLE u(id int);"[..]).unwrap();
    assert_eq!(a, a2, "same content, same blob");
    assert_ne!(a, b);
    assert_eq!(a.len(), 64);
    let mut text = String::new();
    blobs.open(&a).unwrap().read_to_string(&mut text).unwrap();
    assert_eq!(text, "CREATE TABLE t(id int);");
    let keep: HashSet<String> = [a.clone()].into();
    let freed = blobs.remove_unreferenced(&keep).unwrap();
    assert_eq!(freed.count, 1);
    assert!(freed.bytes > 0);
    assert!(blobs.open(&a).is_ok());
    assert!(blobs.open(&b).is_err());
    assert!(blobs.open("../escape").is_err(), "ids are hex digests only");
}

#[test]
fn names_db_matches_whole_identifiers_in_any_case() {
    assert!(names_db("CREATE TABLE mi_a.q (id int)", "mi_a"));
    assert!(names_db("drop table `MI_A`.`q`", "mi_a"));
    assert!(!names_db("CREATE TABLE mi_ab.q (id int)", "mi_a"));
    assert!(!names_db("CREATE TABLE xmi_a.q (id int)", "mi_a"));
    assert!(!names_db("CREATE TABLE q (id int)", "mi_a"));
}

#[test]
fn query_events_under_another_default_db_are_misplaced() {
    let db = "mi_a";
    assert!(!query_misplaced(
        "use `mi_a`; CREATE TABLE mi_a.q (id int)",
        db
    ));
    assert!(!query_misplaced("use `mi_a`; CREATE TABLE q (id int)", db));
    assert!(
        query_misplaced("CREATE TABLE mi_a.q (id int)", db),
        "no default db"
    );
    assert!(query_misplaced(
        "use `information_schema`; CREATE TABLE `mi_a`.q (id int)",
        db
    ));
    assert!(!query_misplaced("use `mi_b`; CREATE TABLE q (id int)", db));
    assert!(!query_misplaced("BEGIN", db));
}

#[test]
fn tracked_saves_are_the_base_and_the_own_segment() {
    let base = Manifest::empty(server()).then(seg(
        "mi_old",
        pos("binlog.000002", 4),
        pos("binlog.000002", 90),
    ));
    let t = Tracked::new("mi_a", base.clone(), pos("binlog.000003", 10));
    assert_eq!(t.checked, t.start);
    let m = t.manifest_at(pos("binlog.000003", 50));
    assert_eq!(
        m,
        base.then(seg(
            "mi_a",
            pos("binlog.000003", 10),
            pos("binlog.000003", 50)
        ))
    );
    let needed: Vec<_> = t.needed().cloned().collect();
    assert_eq!(needed, [pos("binlog.000002", 4), pos("binlog.000003", 10)]);
    assert_eq!(
        t.manifest_at(t.start.clone()),
        base,
        "no changes, no segment"
    );
}

#[test]
fn tracked_rebased_on_a_dump_starts_at_its_position() {
    let t = Tracked::new("mi_a", Manifest::empty(server()), pos("binlog.000003", 10));
    let d = Dumped {
        blob: "cd".repeat(32),
        at: pos("binlog.000004", 7),
    };
    let r = t.rebased(d.clone());
    assert_eq!(r.base, Manifest::rooted(server(), d.blob));
    assert_eq!((r.start.clone(), r.checked.clone()), (d.at.clone(), d.at));
    assert_eq!(r.db, "mi_a");
    assert_eq!(r.needed().count(), 1, "a rooted base needs no binlog");
}

#[test]
fn saves_in_lists_folders_with_manifests_only() {
    let dir = scratch();
    let m = Manifest::empty(server());
    m.write(&dir.path().join("s1")).unwrap();
    m.write(&dir.path().join(".base")).unwrap();
    std::fs::create_dir_all(dir.path().join("no_manifest")).unwrap();
    let mut got = saves_in(dir.path()).unwrap();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(got, [("s1".to_string(), m)]);
    std::fs::create_dir_all(dir.path().join("s3")).unwrap();
    std::fs::write(dir.path().join("s3").join("manifest.json"), "{").unwrap();
    assert!(
        saves_in(dir.path()).is_err(),
        "a broken manifest is an error"
    );
    assert!(saves_in(&dir.path().join("missing")).unwrap().is_empty());
}

#[test]
fn restores_need_the_same_server_and_its_binlog() {
    let here = server();
    assert!(check_same_server("s1", &here, &here).is_ok());
    let other = ServerRef {
        container: "other".into(),
        ..server()
    };
    let e = check_same_server("s1", &other, &here).unwrap_err();
    assert_eq!(e.code, -32001);
    assert!(e.message.contains("other"));
    let replaced = ServerRef {
        id: "new".into(),
        ..server()
    };
    let e = check_same_server("s1", &replaced, &here).unwrap_err();
    assert!(e.message.contains("replaced"), "{}", e.message);
    let purged = restore_error("s1", Purged("binlog.000001".into()).into());
    assert_eq!(purged.code, -32001);
    assert_eq!(restore_error("s1", anyhow::anyhow!("boom")).code, -32603);
    let e = chain_error(
        "i1",
        Misplaced("ALTER TABLE mi_i1.t ADD c int".into()).into(),
    );
    assert_eq!(e.code, -32001);
    assert!(e.message.contains("another database was selected") && e.message.contains("i1"));
    assert_eq!(chain_error("i1", anyhow::anyhow!("down")).code, -32603);
}

fn other_server() -> ServerRef {
    ServerRef {
        container: "old".into(),
        id: "an-old-identity".into(),
        ..server()
    }
}

#[test]
fn a_rooted_base_is_moved_to_this_server_and_its_saves_follow() {
    let base = Manifest::rooted(other_server(), "ab".repeat(32));
    let here = base
        .rehomed(server())
        .expect("a base without segments moves");
    assert_eq!(here.root, base.root);
    let t = Tracked::new("eb_i1", here, pos("binlog.000003", 10));
    let save = t.manifest_at(pos("binlog.000003", 90));
    assert_eq!(save.server, server(), "the save carries this server");
    let r = t.rebased(Dumped {
        blob: "cd".repeat(32),
        at: pos("binlog.000004", 1),
    });
    assert_eq!(r.base.server, server());
    assert_eq!(
        save.rehomed(other_server()),
        None,
        "a save with segments stays put"
    );
}

#[test]
fn tracked_changes_apply_checked_or_the_whole_tracking() {
    let mut t = Tracked::new("mi_a", Manifest::empty(server()), pos("binlog.000003", 10));
    TrackedChange::Checked(pos("binlog.000003", 50)).apply(&mut t);
    assert_eq!((t.start.pos, t.checked.pos), (10, 50));
    let r = t.rebased(Dumped {
        blob: "cd".repeat(32),
        at: pos("binlog.000004", 7),
    });
    TrackedChange::Rebased(r.clone()).apply(&mut t);
    assert_eq!(t, r);
}

#[test]
fn used_roots_and_needed_positions_cover_saves_and_live_databases() {
    let rooted = |c: &str| Manifest::rooted(server(), c.repeat(64));
    let saves = vec![
        (
            "s1".to_string(),
            rooted("a").then(seg(
                "mi_a",
                pos("binlog.000002", 5),
                pos("binlog.000002", 9),
            )),
        ),
        (
            "s2".to_string(),
            Manifest::empty(other_server()).then(seg(
                "mi_b",
                pos("binlog.000001", 4),
                pos("binlog.000001", 8),
            )),
        ),
    ];
    let base = rooted("b");
    let used = used_roots(&saves, [&base]);
    assert_eq!(used, HashSet::from(["a".repeat(64), "b".repeat(64)]));
    let live = Tracked::new("mi_c", rooted("c"), pos("binlog.000003", 1));
    let needed = needed_positions(&server(), &saves, [&live]);
    assert_eq!(
        needed,
        [pos("binlog.000002", 5), pos("binlog.000003", 1)],
        "other servers' saves don't count"
    );
}

/// A standing save `id` (kept by `base_*`) holding the binlog from `from`.
fn standing(id: &str, from: Position) -> (String, Manifest) {
    let m = Manifest::rooted(server(), "a".repeat(64));
    (
        id.to_string(),
        m.then(seg("mi_a", from, pos("binlog.000003", 9))),
    )
}

fn reroot_picks(saves: &[(String, Manifest)], live: Vec<Position>, limit: u64) -> Vec<String> {
    let (root, keep) = (
        Path::new("/nowhere"),
        ["base_*".to_string(), "s9".to_string()],
    );
    let s = Standing {
        root,
        saves,
        keep: &keep,
        live,
        limit,
        scratch: "x",
    };
    let picked = to_reroot(&server(), &s, &files(), &pos("binlog.000003", 50));
    picked.into_iter().map(|(id, _)| id.clone()).collect()
}

#[test]
fn only_saves_kept_by_a_pattern_and_past_the_limit_are_rerooted() {
    let saves = [
        standing("base_x", pos("binlog.000001", 4)),
        standing("base_y", pos("binlog.000003", 40)),
    ];
    assert_eq!(reroot_picks(&saves, vec![], 10), ["base_x", "base_y"]);
    assert!(
        reroot_picks(&saves, vec![], 1_000_000).is_empty(),
        "under the limit"
    );
    // A live database needs the binlog from before them: dumping them frees nothing.
    assert!(reroot_picks(&saves, vec![pos("binlog.000001", 2)], 0).is_empty());
    // A save kept by name goes with its run; it needs the same binlog, so nothing is freed.
    let named = [saves[0].clone(), standing("s9", pos("binlog.000001", 2))];
    assert!(reroot_picks(&named, vec![], 0).is_empty());
    // A save on another server, or without a log, holds nothing here.
    let other = Manifest::empty(other_server()).then(seg(
        "mi_b",
        pos("binlog.000001", 4),
        pos("binlog.000001", 8),
    ));
    let rooted = Manifest::rooted(server(), "a".repeat(64));
    let not_held = [
        ("base_o".to_string(), other),
        ("base_r".to_string(), rooted),
    ];
    assert!(reroot_picks(&not_held, vec![], 0).is_empty());
}
