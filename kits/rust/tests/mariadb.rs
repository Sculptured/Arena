//! The MariaDB helper's save logic, without a server: positions, manifests and blobs.
#![cfg(feature = "mariadb")]
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use arena_part::helpers::mariadb::{
    dump_position, names_db, percent_encode, purge_target, query_misplaced, span, span_bytes,
    BinlogFile, Blobs, Manifest, Position, Segment, ServerRef,
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
