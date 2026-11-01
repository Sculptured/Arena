//! Where a part's save folders are: `$ARENA_SAVES/<package>/<save>/`.
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::temp_dir::sweep_stale;
use super::{folder, remove_tree};
use crate::proto::{kept, valid_id, GcResult};
use crate::PartError;

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

    /// The folder of `save`; an unknown save is -32002.
    pub fn require(&self, save: &str) -> Result<PathBuf, PartError> {
        self.find(save)
            .ok_or_else(|| PartError::unknown(format!("no save {save}")))
    }

    /// Removes the folder of `save`, read-only folders too (nothing to do when it has none).
    pub fn remove(&self, save: &str) -> io::Result<()> {
        let dir = self.find(save);
        self.known
            .lock()
            .expect("save folder map lock")
            .remove(save);
        match dir {
            Some(d) => remove_tree(&d),
            None => Ok(()),
        }
    }

    /// Removes the save folders (folders named like a save id) that `keep` does not keep; a
    /// `keep` entry ending in `*` keeps every id with that prefix. Counts them and their bytes.
    pub fn remove_not_kept<S: AsRef<str>>(&self, keep: &[S]) -> io::Result<GcResult> {
        let mut out = GcResult::default();
        let Ok(rd) = std::fs::read_dir(self.root()) else {
            return Ok(out);
        };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            if valid_id(&name) && !kept(keep, &name) && e.path().is_dir() {
                out.bytes += folder::size(&e.path());
                self.remove(&name).map_err(|err| {
                    io::Error::new(err.kind(), format!("removing save {name}: {err}"))
                })?;
                out.dropped += 1;
            }
        }
        Ok(out)
    }
}
