//! The `sculptured` integration: Sculptured's approvals (proposed changes and their decisions) in
//! the arena world. Each instance is a `sculptured serve` of its own on a loopback port, with its
//! data folder under `.live/<instance>/`; its team's Practice agents propose the changes and the
//! tools decide them, all through Sculptured's HTTP API. A save is an online backup of the
//! server's SQLite stores plus a copy of the rest of its data folder; a restore starts a server on
//! a copy of it. Saves restore exactly.
mod api;
mod changes;
mod config;
mod live;
mod poll;
mod saves;
mod server;
mod team;
mod tool_schemas;
mod tools;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::json;

use api::Api;
use changes::ChangeMap;
use config::Config;
use live::{Live, Run};

const PACKAGE: &str = "sculptured";

type R<T> = Result<T, PartError>;

pub struct Sculptured {
    saves: SaveDirs,
    live: Live,
    /// Held by each tool call and save, so an instance does one thing at a time.
    busy: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// A running instance: its record, its config and its server's API.
struct Instance {
    run: Run,
    config: Config,
    api: Api,
}

/// Runs `f` on a thread of its own: copies and backups touch the disk.
async fn blocking<T, F>(f: F) -> R<T>
where
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
    T: Send + 'static,
{
    let done = tokio::task::spawn_blocking(f).await;
    done.map_err(PartError::internal_from)?
        .map_err(PartError::internal_from)
}

impl Sculptured {
    /// Saves under env `ARENA_SAVES`.
    pub fn from_env() -> Sculptured {
        Sculptured::with(SaveDirs::new(PACKAGE))
    }

    /// Saves in `package_folder` (`<ARENA_SAVES>/sculptured`).
    pub fn new(package_folder: PathBuf) -> Sculptured {
        Sculptured::with(SaveDirs::under(package_folder))
    }

    fn with(saves: SaveDirs) -> Sculptured {
        // absolute: `stop` finds the servers by this path on their command lines
        let live = saves.root().join(".live");
        Sculptured {
            live: Live::new(std::path::absolute(&live).unwrap_or(live)),
            saves,
            busy: Mutex::new(HashMap::new()),
        }
    }

    /// The running instance `id`, from its record.
    fn instance(&self, id: &str) -> R<Instance> {
        let unknown = || PartError::unknown(format!("no instance {id}"));
        let run = self.live.run(id).ok_or_else(unknown)?;
        let port = run.port.ok_or_else(unknown)?; // still starting
        Ok(Instance {
            config: Config::parse(&run.config)?,
            api: Api::new(port).map_err(PartError::internal_from)?,
            run,
        })
    }

    fn lock(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut busy = self.busy.lock().expect("busy map lock");
        busy.entry(id.to_string()).or_default().clone()
    }

    fn save_dir(&self, save: &str) -> R<PathBuf> {
        self.saves
            .find(save)
            .ok_or_else(|| PartError::unknown(format!("no save {save}")))
    }

    /// A target's changes: a live instance's from its server, a save's from its folder.
    async fn changes(&self, t: &Target) -> R<ChangeMap> {
        let list = match t {
            Target::Instance { instance } => {
                let inst = self.instance(instance)?;
                changes::fetch(&inst.api, &inst.config.team).await
            }
            Target::Save { save } => saves::changes(&self.save_dir(save)?),
        };
        Ok(changes::by_key(&list.map_err(PartError::internal_from)?))
    }

    /// Kills the server of `id` and removes its folders.
    async fn bring_down(&self, id: &str, run: &Run) -> R<()> {
        let f = self.live.folders(id, run);
        server::kill(f.data())
            .await
            .map_err(PartError::internal_from)?;
        self.live.remove(id, &f.tmp);
        Ok(())
    }

    /// Starts the server of `id` (on a copy of `from`'s data folder) and its team; returns its
    /// port.
    async fn bring_up(&self, id: &str, run: &Run, from: Option<PathBuf>) -> anyhow::Result<u16> {
        let config = Config::parse(&run.config)?;
        let f = self.live.folders(id, run);
        if let Some(dir) = from {
            let data = f.data();
            tokio::task::spawn_blocking(move || saves::restore(&dir, &data)).await??;
        }
        let port = server::launch(&config.binary, &f).await?;
        let up = Run {
            port: Some(port),
            ..run.clone()
        };
        self.live.record(id, &up)?;
        team::ensure_running(&Api::new(port)?, &config.team).await?;
        Ok(port)
    }
}

/// A short scratch folder of its own for a server's runner socket (a Unix socket path is at most
/// 108 bytes, so it can't be under the saves).
fn scratch() -> PathBuf {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("arena-sc-{}-{n}", std::process::id()))
}

#[async_trait]
impl Part for Sculptured {
    async fn describe(&self) -> R<Describe> {
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
            tools: tool_schemas::schemas(),
        })
    }

    async fn start(&self, p: StartParams) -> R<()> {
        Config::parse(&p.config)?;
        let from = match &p.from {
            Some(f) => Some(self.save_dir(&f.save)?),
            None => None,
        };
        let id = &p.instance;
        if let Some(old) = self.live.run(id) {
            self.bring_down(id, &old).await?; // a leftover would keep its old data
        }
        // recorded first, so a server that comes up is never unknown to a restarted program
        let run = Run {
            config: p.config.clone(),
            port: None,
            tmp: scratch(),
        };
        self.live
            .record(id, &run)
            .map_err(PartError::internal_from)?;
        let up = self.bring_up(id, &run, from).await;
        if let Err(e) = up {
            let _ = self.bring_down(id, &run).await;
            return Err(PartError::internal_from(e));
        }
        Ok(())
    }

    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput> {
        let inst = self.instance(&p.instance)?;
        let lock = self.lock(&p.instance);
        let _held = lock.lock().await;
        tools::call(&inst.api, &inst.config.team, &p.name, &p.args).await
    }

    async fn save(&self, p: SaveParams) -> R<Saved> {
        let inst = self.instance(&p.instance)?;
        let lock = self.lock(&p.instance);
        let _held = lock.lock().await;
        let list = changes::fetch(&inst.api, &inst.config.team).await;
        let list = list.map_err(PartError::internal_from)?;
        let data = self.live.folders(&p.instance, &inst.run).data();
        let dir = PathBuf::from(&p.dir);
        let to = dir.clone();
        let bytes = blocking(move || saves::write(&data, &list, &to)).await?;
        self.saves.note(&p.save, &dir);
        Ok(Saved { bytes, exact: true })
    }

    async fn inspect(&self, p: InspectParams) -> R<Docs> {
        Ok(changes::inspect(&self.changes(&p.target).await?, &p.keys))
    }

    async fn diff(&self, p: DiffParams) -> R<Differing> {
        let (a, b) = (self.changes(&p.a).await?, self.changes(&p.b).await?);
        Ok(Differing::between(&a, &b))
    }

    async fn drop_save(&self, p: SaveId) -> R<()> {
        self.saves.remove(&p.save).map_err(PartError::internal_from)
    }

    async fn stop(&self, p: IdParams) -> R<()> {
        let id = &p.instance;
        let run = self
            .live
            .run(id)
            .ok_or_else(|| PartError::unknown(format!("no instance {id}")))?;
        self.bring_down(id, &run).await?;
        self.busy.lock().expect("busy map lock").remove(id);
        Ok(())
    }

    /// Nothing is kept outside `ARENA_SAVES` but the runners' scratch folders, which `stop`
    /// removes.
    async fn usage(&self) -> R<Usage> {
        Ok(Usage { bytes: 0 })
    }
}
