//! The folder helper: hard-linked saves, copying restores.
use arena_part::helpers::folder::{copy_save, digest, restore, size};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// A temporary folder removed on drop, even when the test fails.
struct Tmp(PathBuf);

impl Tmp {
    fn new(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!("arena_kit_{}_{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn inode(p: &Path) -> u64 {
    fs::metadata(p).unwrap().ino()
}

#[test]
fn copy_save_links_unchanged() {
    let t = Tmp::new("link");
    let (live, s1, s2) = (t.0.join("live"), t.0.join("s1"), t.0.join("s2"));
    put(&live, "a.txt", "aaaa");
    put(&live, "sub/b.txt", "bb");
    assert_eq!(copy_save(&live, &s1, None).unwrap(), 6);
    put(&live, "c.txt", "ccc");
    let written = copy_save(&live, &s2, Some(&s1)).unwrap();
    assert_eq!(written, 3, "only the new file is written");
    assert_eq!(inode(&s1.join("a.txt")), inode(&s2.join("a.txt")));
    assert_eq!(inode(&s1.join("sub/b.txt")), inode(&s2.join("sub/b.txt")));
    assert_eq!(fs::read_to_string(s2.join("c.txt")).unwrap(), "ccc");
    assert_eq!(size(&s2), 9);
}

#[test]
fn copy_save_copies_changed() {
    let t = Tmp::new("changed");
    let (live, s1, s2) = (t.0.join("live"), t.0.join("s1"), t.0.join("s2"));
    put(&live, "a.txt", "aaaa");
    copy_save(&live, &s1, None).unwrap();
    put(&live, "a.txt", "zzzzz");
    assert_eq!(copy_save(&live, &s2, Some(&s1)).unwrap(), 5);
    assert_ne!(inode(&s1.join("a.txt")), inode(&s2.join("a.txt")));
    assert_eq!(fs::read_to_string(s1.join("a.txt")).unwrap(), "aaaa");
}

#[test]
fn restore_is_a_copy() {
    let t = Tmp::new("restore");
    let (live, s1, back) = (t.0.join("live"), t.0.join("s1"), t.0.join("back"));
    put(&live, "a.txt", "aaaa");
    copy_save(&live, &s1, None).unwrap();
    restore(&s1, &back).unwrap();
    assert_ne!(inode(&s1.join("a.txt")), inode(&back.join("a.txt")));
    put(&back, "a.txt", "changed");
    assert_eq!(fs::read_to_string(s1.join("a.txt")).unwrap(), "aaaa");
    restore(&s1, &back).unwrap();
    assert_eq!(fs::read_to_string(back.join("a.txt")).unwrap(), "aaaa");
}

#[test]
fn digest_sees_content_and_names() {
    let t = Tmp::new("digest");
    let (a, b) = (t.0.join("a"), t.0.join("b"));
    put(&a, "x/f", "one");
    copy_save(&a, &b, None).unwrap();
    let (da, db) = (digest(&a).unwrap(), digest(&b).unwrap());
    assert_eq!((da.bytes, da.files), (3, 1));
    assert_eq!(da, db);
    put(&b, "x/f", "two");
    assert_ne!(digest(&b).unwrap().hash, da.hash);
}

#[test]
fn size_of_missing_is_zero() {
    assert_eq!(size(Path::new("/nonexistent/arena_kit")), 0);
}

#[test]
fn copies_keep_folder_modes() {
    use std::os::unix::fs::PermissionsExt;
    let t = Tmp::new("modes");
    let (live, s1, back) = (t.0.join("live"), t.0.join("s1"), t.0.join("back"));
    put(&live, "locked/f", "x");
    fs::set_permissions(live.join("locked"), fs::Permissions::from_mode(0o550)).unwrap();
    copy_save(&live, &s1, None).unwrap();
    restore(&s1, &back).unwrap();
    for dir in [&s1, &back] {
        let mode = fs::metadata(dir.join("locked"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o550);
    }
    for dir in [&live, &s1, &back] {
        fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o750)).unwrap();
    }
}

#[test]
fn sweep_removes_only_stale_folders() {
    use arena_part::helpers::temp_dir::{sweep_stale, TempDir};
    use std::time::{Duration, SystemTime};
    let t = Tmp::new("sweep");
    let (old, fresh) = (t.0.join("old"), t.0.join("fresh"));
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&fresh).unwrap();
    let day = SystemTime::now() - Duration::from_secs(86400);
    fs::File::open(&old).unwrap().set_modified(day).unwrap();
    sweep_stale(&t.0, Duration::from_secs(3600));
    assert!(!old.exists() && fresh.exists());
    let scratch = TempDir::new_in(&t.0, "x").unwrap();
    assert!(scratch.path().is_dir());
    sweep_stale(&t.0.join("missing"), Duration::ZERO);
}
