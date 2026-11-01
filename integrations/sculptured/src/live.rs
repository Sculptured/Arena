//! Running instances on disk: `<package saves>/.live/<instance>/` holds the server's data folder
//! (`data/`), its home (`home/`), its log (`serve.log`), `run.json`, the record a restarted
//! program reads to reach, save and stop the instance, and `aliases.json`, its re-raised changes.
//! A dot folder: arenad's sweep and `gc`'s save removal leave it alone.
use std::io;
use std::path::{Path, PathBuf};

use arena_part::proto::valid_id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::aliases::Aliases;
use crate::server::{Folders, Pids};

/// What a running instance is: its config, the program and build it runs, its server's port
/// once it listens, the scratch folder its runner's socket goes in, and its processes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub config: Value,
    pub binary: PathBuf,
    pub build: String,
    pub port: Option<u16>,
    pub tmp: PathBuf,
    #[serde(default)]
    pub pids: Pids,
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
        write_json(&self.record_path(instance), run)
    }

    /// The record of `instance`, when it runs (or is starting).
    pub fn run(&self, instance: &str) -> Option<Run> {
        read_json(&self.record_path(instance))
    }

    fn aliases_path(&self, instance: &str) -> PathBuf {
        self.folder(instance).join("aliases.json")
    }

    pub fn aliases(&self, instance: &str) -> Aliases {
        read_json(&self.aliases_path(instance)).unwrap_or_default()
    }

    pub fn set_aliases(&self, instance: &str, aliases: &Aliases) -> io::Result<()> {
        write_json(&self.aliases_path(instance), aliases)
    }

    /// Every instance with a folder here.
    pub fn ids(&self) -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(&self.root) else {
            return vec![];
        };
        let names = rd.filter_map(Result::ok).map(|e| e.file_name());
        let names = names.filter_map(|n| n.to_str().map(str::to_string));
        names.filter(|n| valid_id(n)).collect()
    }

    /// Removes the instance's folder and its scratch folder, if known (errors are ignored).
    pub fn remove(&self, instance: &str, tmp: Option<&Path>) {
        let _ = std::fs::remove_dir_all(self.folder(instance));
        if let Some(t) = tmp {
            let _ = std::fs::remove_dir_all(t);
        }
    }
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> io::Result<()> {
    std::fs::create_dir_all(path.parent().expect("a record has a folder"))?;
    std::fs::write(path, serde_json::to_vec(v)?)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
