//! The config of each running instance, kept on disk (`<package saves>/.live/<instance>.json`)
//! so a program that was restarted can still save, inspect and stop it.
use std::io;
use std::path::PathBuf;

use serde_json::Value;

pub struct Live {
    dir: PathBuf,
}

impl Live {
    pub fn new(dir: PathBuf) -> Live {
        Live { dir }
    }

    fn path(&self, instance: &str) -> PathBuf {
        self.dir.join(format!("{instance}.json"))
    }

    pub fn record(&self, instance: &str, config: &Value) -> io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        std::fs::write(self.path(instance), config.to_string())
    }

    /// The config `instance` was started with, when it is running.
    pub fn config(&self, instance: &str) -> Option<Value> {
        let text = std::fs::read_to_string(self.path(instance)).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn forget(&self, instance: &str) {
        let _ = std::fs::remove_file(self.path(instance));
    }
}
