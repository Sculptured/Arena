//! The tar digest: order-free, sensitive to content and metadata.
#![cfg(feature = "compose")]
use arena_part::helpers::tar_digest::digest;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A temporary folder removed on drop, even when the test fails.
struct Tmp(PathBuf);

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tar(dir: &Path, out: &str, names: &[&str]) -> PathBuf {
    let path = dir.join(out);
    let status = Command::new("tar")
        .arg("-C")
        .arg(dir.join("src"))
        .arg("-cf")
        .arg(&path)
        .args(names)
        .status()
        .unwrap();
    assert!(status.success());
    path
}

#[test]
fn digest_ignores_order_and_sees_changes() {
    let t = Tmp(std::env::temp_dir().join(format!("arena_kit_tar_{}", std::process::id())));
    fs::create_dir_all(t.0.join("src")).unwrap();
    fs::write(t.0.join("src/a"), "aaaa").unwrap();
    fs::write(t.0.join("src/b"), "bb").unwrap();
    let ab = digest(&tar(&t.0, "ab.tar", &["a", "b"])).unwrap();
    let ba = digest(&tar(&t.0, "ba.tar", &["b", "a"])).unwrap();
    assert_eq!((ab.files, ab.bytes), (2, 6));
    assert_eq!(ab, ba);
    fs::write(t.0.join("src/b"), "cc").unwrap();
    assert_ne!(
        digest(&tar(&t.0, "c.tar", &["a", "b"])).unwrap().hash,
        ab.hash
    );
    let touched = Command::new("touch")
        .args(["-d", "@1700000000"])
        .arg(t.0.join("src/a"))
        .status()
        .unwrap();
    assert!(touched.success());
    let moved = digest(&tar(&t.0, "d.tar", &["a"])).unwrap();
    assert_ne!(
        moved.hash,
        digest(&tar(&t.0, "e.tar", &["b"])).unwrap().hash
    );
}
