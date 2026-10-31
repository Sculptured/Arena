//! A scratch folder that goes away when dropped.
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
