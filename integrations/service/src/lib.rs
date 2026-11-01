//! The `service` integration: each instance is a Docker Compose project `arena_<instance>`, and a
//! save holds copies of the volumes the config names.
//!
//! A save stops the project, writes each volume to `<save>/<volume>.tar` exactly as the helper
//! container streams it (owners, modes and mtimes kept) and starts it again, so it is exact.
//! Starting from a save makes a new project whose volumes are filled from the archives. Each
//! running instance's config is kept in `.live/<instance>.json`, so a restarted program still
//! knows its instances. Live volumes are not counted by `usage`: `stop` removes them.
mod config;
mod live;
mod view;
mod volumes;

use std::path::{Path, PathBuf};

use arena_part::helpers::blocking;
use arena_part::helpers::compose::Project;
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::helpers::temp_dir::TempDir;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use config::Config;
use live::Live;
use serde_json::json;
use volumes::Instance;

const PACKAGE: &str = "service";

pub struct Service {
    saves: SaveDirs,
    live: Live,
}

impl Service {
    /// Saves under env `ARENA_SAVES`.
    pub fn from_env() -> Service {
        let saves = SaveDirs::new(PACKAGE);
        saves.sweep_scratch();
        Service::with(saves)
    }

    /// Saves in `package_folder` (`<ARENA_SAVES>/service`).
    pub fn new(package_folder: PathBuf) -> Service {
        Service::with(SaveDirs::under(package_folder))
    }

    fn with(saves: SaveDirs) -> Service {
        Service {
            live: Live::new(saves.root().join(".live")),
            saves,
        }
    }

    /// The running instance `id`, rebuilt from the config recorded when it started.
    fn instance(&self, id: &str) -> Result<Instance, PartError> {
        let config = self
            .live
            .config(id)
            .ok_or_else(|| PartError::unknown(format!("no instance {id}")))?;
        new_instance(id, &Config::parse(&config)?)
    }

    /// The digests of a target's volumes: a save's folders, or a copy of a live instance's volumes.
    async fn digests(&self, t: &Target) -> Result<view::DigestMap, PartError> {
        match t {
            Target::Save { save } => view::digests(&self.saves.require(save)?),
            Target::Instance { instance } => {
                let (inst, scratch) = (self.instance(instance)?, self.saves.scratch());
                blocking(move || {
                    let tmp =
                        TempDir::new_in(&scratch, "inspect").map_err(PartError::internal_from)?;
                    volumes::export_all(&inst, tmp.path()).map_err(PartError::internal_from)?;
                    view::digests(tmp.path())
                })
                .await
            }
        }
    }
}

/// The instance `id` with `config`, its project not yet run.
fn new_instance(id: &str, config: &Config) -> Result<Instance, PartError> {
    let name = format!("arena_{id}");
    let project =
        Project::new(&name, &config.compose_file, &config.env).map_err(PartError::internal_from)?;
    let project = project
        .with_helper(config.helper())
        .map_err(PartError::internal_from)?;
    Ok(Instance {
        project,
        volumes: config.volumes.clone(),
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
        PartError::internal_from(e)
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
        let inst = new_instance(&p.instance, &Config::parse(&p.config)?)?;
        let from = match &p.from {
            Some(f) => Some(self.saves.require(&f.save)?),
            None => None,
        };
        // recorded first, so a project that comes up is never unknown to a restarted program
        let id = &p.instance;
        self.live
            .record(id, &p.config)
            .map_err(PartError::internal_from)?;
        let up = blocking(move || bring_up(&inst, from.as_deref())).await;
        if up.is_err() {
            self.live.forget(id);
        }
        up
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
        let dir = PathBuf::from(&p.dir);
        let to = dir.clone();
        let bytes =
            blocking(move || volumes::save_all(&inst, &to).map_err(PartError::internal_from))
                .await?;
        self.saves.note(&p.save, &dir);
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
        self.saves.remove(&p.save).map_err(PartError::internal_from)
    }

    async fn stop(&self, p: IdParams) -> Result<(), PartError> {
        let inst = self.instance(&p.instance)?;
        blocking(move || {
            inst.project
                .down_with_volumes()
                .map_err(PartError::internal_from)
        })
        .await?;
        self.live.forget(&p.instance);
        Ok(())
    }

    /// Nothing is kept outside `ARENA_SAVES`: live volumes are removed by `stop`.
    async fn usage(&self) -> Result<Usage, PartError> {
        Ok(Usage { bytes: 0 })
    }
}
