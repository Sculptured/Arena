//! The `service` integration: each instance is a Docker Compose project `arena_<instance>`, and a
//! save holds copies of the volumes the config names.
//!
//! A save stops the project, copies the volumes to `<save>/<volume>/` (hard-linking files that
//! have not changed since the instance's last save) and starts it again, so it is exact. Starting
//! from a save makes a new project whose volumes are filled from the save. Live volumes are not
//! counted by `usage`: `stop` removes them.
mod config;
mod view;
mod volumes;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use arena_part::helpers::compose::Project;
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::helpers::temp_dir::TempDir;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use config::Config;
use serde_json::json;
use volumes::Instance;

const PACKAGE: &str = "service";

pub struct Service {
    saves: SaveDirs,
    live: Mutex<HashMap<String, Instance>>,
}

fn internal(e: impl std::fmt::Display) -> PartError {
    PartError::internal(format!("{e:#}"))
}

/// Runs `f` on a thread of its own: Docker calls take seconds.
async fn blocking<T, F>(f: F) -> Result<T, PartError>
where
    F: FnOnce() -> Result<T, PartError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(internal)?
}

impl Service {
    /// Saves under env `ARENA_SAVES`.
    pub fn from_env() -> Service {
        Service::with(SaveDirs::new(PACKAGE))
    }

    /// Saves in `package_folder` (`<ARENA_SAVES>/service`).
    pub fn new(package_folder: PathBuf) -> Service {
        Service::with(SaveDirs::under(package_folder))
    }

    fn with(saves: SaveDirs) -> Service {
        Service {
            saves,
            live: Mutex::new(HashMap::new()),
        }
    }

    fn instances(&self) -> MutexGuard<'_, HashMap<String, Instance>> {
        self.live.lock().expect("instance map lock")
    }

    fn instance(&self, id: &str) -> Result<Instance, PartError> {
        match self.instances().get(id) {
            Some(i) => Ok(i.clone()),
            None => Err(PartError::unknown(format!("no instance {id}"))),
        }
    }

    fn save_dir(&self, save: &str) -> Result<PathBuf, PartError> {
        self.saves
            .find(save)
            .ok_or_else(|| PartError::unknown(format!("no save {save}")))
    }

    fn scratch(&self) -> PathBuf {
        self.saves.root().join(".tmp")
    }

    /// The digests of a target's volumes: a save's folders, or a copy of a live instance's volumes.
    async fn digests(&self, t: &Target) -> Result<view::DigestMap, PartError> {
        match t {
            Target::Save { save } => view::digests(&self.save_dir(save)?),
            Target::Instance { instance } => {
                let (inst, scratch) = (self.instance(instance)?, self.scratch());
                blocking(move || {
                    let tmp = TempDir::new_in(&scratch, "inspect").map_err(internal)?;
                    volumes::export_all(&inst, tmp.path()).map_err(internal)?;
                    view::digests(tmp.path())
                })
                .await
            }
        }
    }
}

/// The instance `p` describes, its project not yet run.
fn new_instance(p: &StartParams, config: &Config) -> Result<Instance, PartError> {
    let name = format!("arena_{}", p.instance);
    let project = Project::new(&name, &config.compose_file, &config.env).map_err(internal)?;
    Ok(Instance {
        project: project.with_helper(config.helper()),
        volumes: config.volumes.clone(),
        prev_save: None,
    })
}

/// Starts the project fresh, or from the folders of `save`; a failure leaves nothing running.
fn bring_up(inst: &Instance, save: Option<&Path>) -> Result<(), PartError> {
    let start = || {
        inst.project.down_with_volumes()?; // a leftover project would keep its old volumes
        match save {
            Some(dir) => volumes::restore_all(inst, dir),
            None => inst.project.up(),
        }
    };
    start().map_err(|e| {
        let _ = inst.project.down_with_volumes();
        internal(e)
    })
}

#[async_trait]
impl Part for Service {
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
            tools: vec![],
        })
    }

    async fn start(&self, p: StartParams) -> Result<(), PartError> {
        let mut inst = new_instance(&p, &Config::parse(&p.config)?)?;
        let from = match &p.from {
            Some(f) => Some(self.save_dir(&f.save)?),
            None => None,
        };
        inst.prev_save = from.clone();
        let up = inst.clone();
        blocking(move || bring_up(&up, from.as_deref())).await?;
        self.instances().insert(p.instance, inst);
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> Result<ToolOutput, PartError> {
        self.instance(&p.instance)?;
        Ok(ToolOutput {
            error: Some(format!("the service integration has no tool {}", p.name)),
            ..ToolOutput::default()
        })
    }

    async fn save(&self, p: SaveParams) -> Result<Saved, PartError> {
        let inst = self.instance(&p.instance)?;
        let (dir, scratch) = (PathBuf::from(&p.dir), self.scratch());
        let to = dir.clone();
        let bytes =
            blocking(move || volumes::save_all(&inst, &to, &scratch).map_err(internal)).await?;
        self.saves.note(&p.save, &dir);
        if let Some(i) = self.instances().get_mut(&p.instance) {
            i.prev_save = Some(dir);
        }
        Ok(Saved { bytes, exact: true })
    }

    async fn inspect(&self, p: InspectParams) -> Result<Docs, PartError> {
        Ok(view::inspect(&self.digests(&p.target).await?, &p.keys))
    }

    async fn diff(&self, p: DiffParams) -> Result<Differing, PartError> {
        let (a, b) = (self.digests(&p.a).await?, self.digests(&p.b).await?);
        Ok(Differing {
            differing: view::diff(&a, &b),
        })
    }

    async fn drop_save(&self, p: SaveId) -> Result<(), PartError> {
        self.saves.remove(&p.save).map_err(internal)
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        let inst = self.instance(&p.instance)?;
        blocking(move || inst.project.down_with_volumes().map_err(internal)).await?;
        self.instances().remove(&p.instance);
        Ok(())
    }

    /// Nothing is kept outside `ARENA_SAVES`: live volumes are removed by `stop`.
    async fn usage(&self) -> Result<Usage, PartError> {
        Ok(Usage { bytes: 0 })
    }
}
