//! A scratch folder that goes away when dropped.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

pub struct TempDir(PathBuf);

impl TempDir {
    /// A new empty folder `<parent>/<tag>_<pid>_<n>`.
    pub fn new_in(parent: &Path, tag: &str) -> io::Result<TempDir> {
        static COUNT: AtomicU64 = AtomicU64::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!("{tag}_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(TempDir(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Removes the folders in `parent` last modified more than `max_age` ago (left by a crashed
/// program); other entries and errors are ignored.
pub fn sweep_stale(parent: &Path, max_age: Duration) {
    let Ok(rd) = std::fs::read_dir(parent) else {
        return;
    };
    let now = SystemTime::now();
    for entry in rd.filter_map(Result::ok) {
        let age = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok());
        if entry.path().is_dir() && age.is_some_and(|a| a > max_age) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
