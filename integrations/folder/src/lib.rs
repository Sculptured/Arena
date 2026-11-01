//! The `folder` integration: each instance is a folder of files, each save a copy of it.
//!
//! Live folders are `<package saves>/.live/<instance>`; the leading dot keeps them from being
//! taken for a save folder. A save hard-links the files unchanged since the instance's previous save.
mod docs;
mod paths;
mod tools;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::helpers::{folder, remove_tree};
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

const PACKAGE: &str = "folder";

/// A live instance: its folder and where its last save went, to link against.
struct Instance {
    dir: PathBuf,
    prev_save: Option<PathBuf>,
}

pub struct Folder {
    saves: SaveDirs,
    live_root: PathBuf,
    live: Mutex<HashMap<String, Instance>>,
}

impl Folder {
    /// Saves under env `ARENA_SAVES`.
    pub fn from_env() -> Folder {
        let saves = SaveDirs::new(PACKAGE);
        saves.sweep_scratch();
        Folder::with(saves)
    }

    /// Saves in `package_folder` (`<ARENA_SAVES>/folder`).
    pub fn new(package_folder: PathBuf) -> Folder {
        Folder::with(SaveDirs::under(package_folder))
    }

    fn with(saves: SaveDirs) -> Folder {
        Folder {
            live_root: saves.root().join(".live"),
            saves,
            live: Mutex::new(HashMap::new()),
        }
    }

    fn instances(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instance>> {
        self.live.lock().expect("instance map lock")
    }

    fn instance_dir(&self, id: &str) -> Result<PathBuf, PartError> {
        match self.instances().get(id) {
            Some(i) => Ok(i.dir.clone()),
            None => Err(PartError::unknown(format!("no instance {id}"))),
        }
    }

    fn target_dir(&self, t: &Target) -> Result<PathBuf, PartError> {
        match t {
            Target::Instance { instance } => self.instance_dir(instance),
            Target::Save { save } => self.saves.require(save),
        }
    }

    /// Fills `dir` from a save, from the `root` the config names, or leaves it empty.
    fn fill(&self, dir: &Path, p: &StartParams) -> Result<(), PartError> {
        if let Some(from) = &p.from {
            return folder::restore(&self.saves.require(&from.save)?, dir)
                .map_err(PartError::internal_from);
        }
        match p.config.get("root").and_then(Value::as_str) {
            Some(seed) if Path::new(seed).is_dir() => {
                folder::restore(Path::new(seed), dir).map_err(PartError::internal_from)
            }
            Some(seed) => Err(PartError::invalid(format!("root {seed:?} is not a folder"))),
            None => std::fs::create_dir_all(dir).map_err(PartError::internal_from),
        }
    }
}

#[async_trait]
impl Part for Folder {
    async fn describe(&self) -> Result<Describe, PartError> {
        Ok(Describe {
            name: PACKAGE.into(),
            version: env!("CARGO_PKG_VERSION").into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: false,
            fingerprint: json!({"integration": PACKAGE}),
            tools: tools::schemas(),
        })
    }

    async fn start(&self, p: StartParams) -> Result<(), PartError> {
        let dir = self.live_root.join(&p.instance);
        folder::discard(&dir);
        if let Err(e) = self.fill(&dir, &p) {
            folder::discard(&dir);
            return Err(e);
        }
        let prev_save = p.from.as_ref().and_then(|f| self.saves.find(&f.save));
        self.instances()
            .insert(p.instance, Instance { dir, prev_save });
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        let dir = self.instance_dir(&p.instance)?;
        Ok(tools::run(&dir, &p.name, &p.args))
    }

    async fn save(&self, p: SaveParams) -> Result<Saved, PartError> {
        let dir = self.instance_dir(&p.instance)?;
        let prev = self
            .instances()
            .get(&p.instance)
            .and_then(|i| i.prev_save.clone());
        let to = PathBuf::from(&p.dir);
        let bytes = folder::copy_save(&dir, &to, prev.as_deref()).map_err(|e| {
            folder::discard(&to); // no half-written save
            PartError::internal_from(e)
        })?;
        self.saves.note(&p.save, &to);
        if let Some(i) = self.instances().get_mut(&p.instance) {
            i.prev_save = Some(to);
        }
        Ok(Saved { bytes, exact: true })
    }

    async fn inspect(&self, p: InspectParams) -> Result<Docs, PartError> {
        docs::inspect(&self.target_dir(&p.target)?, &p.keys)
    }

    async fn diff(&self, p: DiffParams) -> Result<Differing, PartError> {
        let (a, b) = (self.target_dir(&p.a)?, self.target_dir(&p.b)?);
        Ok(Differing {
            differing: docs::diff(&a, &b)?,
        })
    }

    async fn drop_save(&self, p: SaveId) -> Result<(), PartError> {
        self.saves.remove(&p.save).map_err(PartError::internal_from)
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        let dir = self.instance_dir(&p.instance)?;
        remove_tree(&dir)
            .map_err(|e| PartError::internal_from(format!("cannot remove {dir:?}: {e}")))?;
        self.instances().remove(&p.instance);
        Ok(())
    }

    /// Everything the part keeps is inside `ARENA_SAVES`, which arenad measures.
    async fn usage(&self) -> Result<Usage, PartError> {
        Ok(Usage { bytes: 0 })
    }
}
