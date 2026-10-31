//! Paths a tool names, kept inside an instance's folder.
use std::path::{Component, Path, PathBuf};

/// A path inside the instance folder: its key form (`a/b`, no `.` parts) and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inside {
    pub rel: String,
    pub abs: PathBuf,
}

/// The names in `rel`, or why it can't be used: absolute paths and `..` are refused.
fn names(rel: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for c in Path::new(rel).components() {
        match c {
            Component::Normal(n) => out.push(n.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(format!("path {rel:?} leaves the instance folder")),
        }
    }
    Ok(out)
}

/// The nearest ancestor of `path` (or `path`) that exists, even as a dangling symlink.
fn nearest_existing(path: &Path) -> Option<&Path> {
    path.ancestors()
        .find(|p| std::fs::symlink_metadata(p).is_ok())
}

/// Refuses a path whose existing part resolves, through symlinks, outside `root`.
fn check_resolved(root: &Path, abs: &Path) -> Result<(), String> {
    let escape = || format!("path {abs:?} leaves the instance folder");
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let probe = nearest_existing(abs).ok_or_else(escape)?;
    match probe.canonicalize() {
        Ok(real) if real.starts_with(&root) => Ok(()),
        _ => Err(escape()),
    }
}

/// `rel` inside `root`, with `""` and `.` meaning `root` itself when `allow_root` is set.
pub fn resolve(root: &Path, rel: &str, allow_root: bool) -> Result<Inside, String> {
    let parts = names(rel)?;
    if parts.is_empty() && !allow_root {
        return Err("a file path is needed".into());
    }
    let abs = parts.iter().fold(root.to_path_buf(), |p, n| p.join(n));
    check_resolved(root, &abs)?;
    Ok(Inside {
        rel: parts.join("/"),
        abs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_parent_and_absolute() {
        let root = std::env::temp_dir();
        for bad in ["../x", "a/../b", "/etc/x"] {
            assert!(resolve(&root, bad, true).is_err(), "{bad}");
        }
        assert!(resolve(&root, "", false).is_err());
        assert_eq!(resolve(&root, "./a/./b", false).unwrap().rel, "a/b");
        assert_eq!(resolve(&root, ".", true).unwrap().rel, "");
    }
}
