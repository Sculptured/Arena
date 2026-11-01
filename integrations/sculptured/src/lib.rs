//! The `sculptured` integration: Sculptured's approvals (proposed changes and their decisions) in
//! the arena world. Each instance is a `sculptured serve` of its own on a loopback port, with its
//! data folder under `.live/<instance>/`; its team's Practice agents propose the changes and the
//! tools decide them, all through Sculptured's HTTP API. A save is an online backup of the
//! server's SQLite stores plus a copy of the rest of its data folder; a restore starts a server on
//! a copy of it and proposes again the changes that waited (see `aliases`). Saves restore exactly.
mod aliases;
mod api;
mod changes;
mod config;
mod instances;
mod live;
mod poll;
mod reraise;
mod saves;
mod server;
mod team;
mod tool_schemas;
mod tools;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arena_part::helpers::blocking;
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::json;

pub use config::default_binary;

use api::Api;
use changes::ChangeMap;
use config::Config;
use live::{Live, Run};
use saves::Meta;
use tools::Team;

const PACKAGE: &str = "sculptured";

type R<T> = Result<T, PartError>;

pub struct Sculptured {
    saves: SaveDirs,
    live: Live,
    /// Held by each tool call and save, so an instance does one thing at a time.
    busy: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The instances this value started and has not stopped: `gc` leaves them alone.
    mine: Mutex<HashSet<String>>,
}

/// A running instance: its record, its config and its server's API.
struct Instance {
    run: Run,
    config: Config,
    api: Api,
}

impl Instance {
    /// The instance's team for the tools, with its re-raised changes.
    fn team<'a>(&'a self, aliases: &'a aliases::Aliases) -> Team<'a> {
        Team {
            api: &self.api,
            name: &self.config.team,
            aliases,
        }
    }
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
        let live = saves.root().join(".live");
        Sculptured {
            live: Live::new(std::path::absolute(&live).unwrap_or(live)),
            saves,
            busy: Mutex::new(HashMap::new()),
            mine: Mutex::new(HashSet::new()),
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

    /// A target's changes as the tools show them: a live instance's from its server, a save's
    /// from its folder.
    async fn changes(&self, t: &Target) -> R<ChangeMap> {
        let list = match t {
            Target::Instance { instance } => {
                let inst = self.instance(instance)?;
                let aliases = self.live.aliases(instance);
                let team = inst.team(&aliases);
                team.changes().await
            }
            Target::Save { save } => saves::changes(&self.saves.require(save)?),
        };
        Ok(changes::by_key(&list.map_err(PartError::internal_from)?))
    }
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
        self.start_instance(p).await
    }

    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput> {
        let inst = self.instance(&p.instance)?;
        let lock = self.lock(&p.instance);
        let _held = lock.lock().await;
        let aliases = self.live.aliases(&p.instance);
        let team = inst.team(&aliases);
        tools::call(&team, &p.name, &p.args).await
    }

    async fn save(&self, p: SaveParams) -> R<Saved> {
        let inst = self.instance(&p.instance)?;
        let lock = self.lock(&p.instance);
        let _held = lock.lock().await;
        let aliases = self.live.aliases(&p.instance);
        let team = inst.team(&aliases);
        let list = team.changes().await.map_err(PartError::internal_from)?;
        let meta = Meta {
            build: inst.run.build.clone(),
            aliases,
        };
        let data = self.live.folders(&p.instance, &inst.run).data();
        let (dir, to) = (PathBuf::from(&p.dir), PathBuf::from(&p.dir));
        let written = blocking(move || saves::write(&data, &list, &meta, &to)).await;
        let bytes = written.map_err(PartError::internal_from)?;
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

    /// A no-op for an instance that is not running (arenad stops instances it is unsure of).
    async fn stop(&self, p: IdParams) -> R<()> {
        self.stop_instance(&p.instance).await
    }

    /// Nothing is kept outside `ARENA_SAVES` but the runners' scratch folders, which `stop`
    /// and `gc` remove.
    async fn usage(&self) -> R<Usage> {
        Ok(Usage { bytes: 0 })
    }

    /// Stops every instance this value did not start (leaked by a program that ended without
    /// stopping them), then drops the saves `keep` does not keep. A failing stop doesn't keep the
    /// rest from being done, and every failure is reported at the end.
    async fn gc(&self, p: GcParams) -> R<GcResult> {
        let stopped = self.stop_leaked().await;
        let removed = self.saves.remove_not_kept(&p.keep);
        match (stopped, removed.map_err(PartError::internal_from)) {
            (Ok(()), removed) => removed,
            (Err(e), Ok(_)) => Err(e),
            (Err(e), Err(r)) => Err(PartError::internal(format!("{}; {}", e.message, r.message))),
        }
    }
}
