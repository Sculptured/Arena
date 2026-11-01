//! The MariaDB helper's save logic, without a server: positions, manifests and blobs.
#![cfg(feature = "mariadb")]
use std::collections::HashSet;
use std::io::Read;
use std::path::PathBuf;

use arena_part::helpers::mariadb::{
    purge_target, span, span_bytes, BinlogFile, Blobs, Manifest, Position, Segment, ServerRef,
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
    assert_eq!(Manifest::read(dir.path()).unwrap(), m);
    assert!(Manifest::read(&dir.path().join("missing")).is_err());
}

#[test]
fn purge_target_is_the_oldest_needed_file_or_the_current_one() {
    let current = pos("binlog.000010", 300);
    let needed = [pos("binlog.000003", 50), pos("binlog.000002", 999)];
    assert_eq!(purge_target(&needed, &current), "binlog.000002");
    assert_eq!(purge_target(&[], &current), "binlog.000010");
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
