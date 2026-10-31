//! Where a part's save folders are: `$ARENA_SAVES/<package>/<save>/`.
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::temp_dir::sweep_stale;

/// Scratch folders older than this are leftovers of a crashed program.
const STALE_AFTER: Duration = Duration::from_secs(3600);

/// The folders of one package's saves. A save that this process made is found where `save` put
/// it; any other (made before a restart) at the convention's path.
pub struct SaveDirs {
    default_root: PathBuf,
    known: Mutex<HashMap<String, PathBuf>>,
}

impl SaveDirs {
    /// Saves of `package` under env `ARENA_SAVES` (default `saves`).
    pub fn new(package: &str) -> SaveDirs {
        let root = std::env::var_os("ARENA_SAVES").map_or_else(|| "saves".into(), PathBuf::from);
        SaveDirs::under(root.join(package))
    }

    /// Saves under `<package folder>/<save>`.
    pub fn under(package_folder: PathBuf) -> SaveDirs {
        SaveDirs {
            default_root: package_folder,
            known: Mutex::new(HashMap::new()),
        }
    }

    /// The folder that holds the saves by convention.
    pub fn root(&self) -> &Path {
        &self.default_root
    }

    /// The folder for scratch copies (`.tmp` next to the saves, never taken for a save).
    pub fn scratch(&self) -> PathBuf {
        self.default_root.join(".tmp")
    }

    /// Removes scratch folders over an hour old; call once when the program starts.
    pub fn sweep_scratch(&self) {
        sweep_stale(&self.scratch(), STALE_AFTER);
    }

    /// Remembers that `save` was written to `dir`.
    pub fn note(&self, save: &str, dir: &Path) {
        self.known
            .lock()
            .expect("save folder map lock")
            .insert(save.to_string(), dir.to_path_buf());
    }

    /// The folder of `save`, when it exists.
    pub fn find(&self, save: &str) -> Option<PathBuf> {
        let known = self.known.lock().expect("save folder map lock");
        let dir = known
            .get(save)
            .cloned()
            .unwrap_or_else(|| self.default_root.join(save));
        dir.is_dir().then_some(dir)
    }

    /// Removes the folder of `save` (nothing to do when it has none).
    pub fn remove(&self, save: &str) -> io::Result<()> {
        let dir = self.find(save);
        self.known
            .lock()
            .expect("save folder map lock")
            .remove(save);
        match dir {
            Some(d) => std::fs::remove_dir_all(d),
            None => Ok(()),
        }
    }
}
