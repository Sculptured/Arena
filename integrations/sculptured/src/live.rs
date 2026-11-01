//! Running instances on disk: `<package saves>/.live/<instance>/` holds the server's data folder
//! (`data/`), its home (`home/`), its log (`serve.log`) and `run.json`, the record a restarted
//! program reads to reach, save and stop the instance. A dot folder: arenad's sweep and `gc`
//! leave it alone.
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::server::Folders;

/// What a running instance is: its config, its server's port once it listens, and the short
/// scratch folder its runner's socket goes in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub config: Value,
    pub port: Option<u16>,
    pub tmp: PathBuf,
}

pub struct Live {
    root: PathBuf,
}

impl Live {
    pub fn new(root: PathBuf) -> Live {
        Live { root }
    }

    fn folder(&self, instance: &str) -> PathBuf {
        self.root.join(instance)
    }

    /// The folders of `instance`'s server, as `run` records them.
    pub fn folders(&self, instance: &str, run: &Run) -> Folders {
        Folders {
            root: self.folder(instance),
            tmp: run.tmp.clone(),
        }
    }

    fn record_path(&self, instance: &str) -> PathBuf {
        self.folder(instance).join("run.json")
    }

    pub fn record(&self, instance: &str, run: &Run) -> io::Result<()> {
        std::fs::create_dir_all(self.folder(instance))?;
        std::fs::write(self.record_path(instance), serde_json::to_vec(run)?)
    }

    /// The record of `instance`, when it runs (or is starting).
    pub fn run(&self, instance: &str) -> Option<Run> {
        let text = std::fs::read(self.record_path(instance)).ok()?;
        serde_json::from_slice(&text).ok()
    }

    /// Removes the instance's folder and its scratch folder (errors are ignored).
    pub fn remove(&self, instance: &str, tmp: &Path) {
        let _ = std::fs::remove_dir_all(self.folder(instance));
        let _ = std::fs::remove_dir_all(tmp);
    }
}
