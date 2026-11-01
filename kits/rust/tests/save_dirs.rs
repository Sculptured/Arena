//! Save folders and busy marks.
#![cfg(feature = "folder")]
use std::path::PathBuf;

use arena_part::helpers::busy::BusySet;
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::helpers::temp_dir::TempDir;

fn scratch() -> TempDir {
    TempDir::new_in(&PathBuf::from(env!("CARGO_TARGET_TMPDIR")), "save_dirs").unwrap()
}

#[test]
fn remove_not_kept_drops_save_folders_only() {
    let tmp = scratch();
    let dirs = SaveDirs::under(tmp.path().to_path_buf());
    for name in ["base_1", "s1", "s2", ".blobs", "Not-An-Id"] {
        std::fs::create_dir_all(tmp.path().join(name)).unwrap();
    }
    std::fs::write(tmp.path().join("s1").join("f"), "12345").unwrap();
    std::fs::write(tmp.path().join("loose_file"), "x").unwrap();
    let out = dirs.remove_not_kept(&["base_*", "s2"]).unwrap();
    assert_eq!((out.dropped, out.bytes), (1, 5));
    let mut left: Vec<String> = std::fs::read_dir(tmp.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, [".blobs", "Not-An-Id", "base_1", "loose_file", "s2"]);
    let none = SaveDirs::under(tmp.path().join("missing"));
    assert_eq!(none.remove_not_kept(&[] as &[&str]).unwrap().dropped, 0);
}

#[test]
fn busy_marks_last_until_dropped() {
    let set = BusySet::default();
    let a = set.mark("a");
    {
        let _b = set.mark("b");
        assert!(set.snapshot().contains("b"));
    }
    assert!(set.snapshot().contains("a") && !set.snapshot().contains("b"));
    assert!(set.try_mark("a").is_none(), "a is busy");
    drop(a);
    assert!(set.snapshot().is_empty());
    let c = set.try_mark("a").expect("a is free again");
    assert!(set.snapshot().contains("a"));
    drop(c);
    assert!(set.snapshot().is_empty());
}
