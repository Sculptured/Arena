//! Removing a folder tree whose folders may be read-only (a copy keeps the modes it copied, and
//! a read-only folder's entries can't be removed).
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Removes `path` and everything under it, giving each folder owner read, write and execute on
/// the way down. A missing path is Ok; a file or a link (never followed) is removed as it is.
pub fn remove_tree(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if !meta.is_dir() {
        return gone_is_ok(fs::remove_file(path));
    }
    let mode = meta.permissions().mode();
    if mode & 0o700 != 0o700 {
        fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700))?;
    }
    for entry in fs::read_dir(path)? {
        remove_tree(&entry?.path())?;
    }
    gone_is_ok(fs::remove_dir(path))
}

/// Something removed meanwhile is removed.
fn gone_is_ok(r: io::Result<()>) -> io::Result<()> {
    match r {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        r => r,
    }
}

/// `remove_tree`, logging a failure to stderr (for cleanups that can't report one).
pub fn remove_tree_or_log(path: &Path) {
    if let Err(e) = remove_tree(path) {
        eprintln!("could not remove {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_read_only_folders_and_missing_paths() {
        let root = std::env::temp_dir().join(format!("arena-tree-{}", std::process::id()));
        let inner = root.join("a/b");
        fs::create_dir_all(&inner).unwrap();
        fs::write(inner.join("f"), "x").unwrap();
        std::os::unix::fs::symlink("/", root.join("a/link")).unwrap();
        for d in [&inner, &root.join("a")] {
            fs::set_permissions(d, fs::Permissions::from_mode(0o500)).unwrap();
        }
        let removed = remove_tree(&root);
        if removed.is_err() {
            let _ = std::process::Command::new("chmod")
                .arg("-R")
                .arg("u+rwx")
                .arg(&root)
                .status();
            let _ = fs::remove_dir_all(&root);
        }
        removed.unwrap();
        assert!(!root.exists());
        remove_tree(&root).unwrap();
    }
}
