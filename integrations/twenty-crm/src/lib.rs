//! The `twenty-crm` integration: Twenty, an open-source CRM. Each instance is a Twenty server with
//! a Redis of its own, on its own database of one Postgres server, so a save is a copy of that
//! database (the kit's Postgres templates) and restores exactly.
mod base;
mod instances;
mod rest;
mod rows;
mod settings;
mod stack;
mod tool_schemas;
mod tools;
mod workspace;

use std::path::PathBuf;

use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::OnceCell;

pub use settings::{Settings, DEFAULT_IMAGE};

use rest::Rest;
use stack::Stack;
use workspace::Creds;

type R<T> = Result<T, PartError>;

/// The database server up and the base made.
struct Ready {
    stack: Stack,
    creds: Creds,
}

pub struct TwentyCrm {
    settings: Settings,
    /// `.base` (the base's sign-in) and `.live` (what runs) are under it.
    root: PathBuf,
    ready: OnceCell<Ready>,
}

impl TwentyCrm {
    /// The server of env `ARENA_TWENTY_*`, files under `$ARENA_SAVES/twenty-crm`.
    pub fn from_env() -> anyhow::Result<TwentyCrm> {
        let root = SaveDirs::new("twenty-crm").root().to_path_buf();
        Ok(TwentyCrm::new(Settings::from_env()?, root))
    }

    pub fn new(settings: Settings, root: PathBuf) -> TwentyCrm {
        TwentyCrm {
            settings,
            root,
            ready: OnceCell::new(),
        }
    }

    /// The server up, on `config` if this is the first call; a later config can't name another.
    async fn ready(&self, config: &Value) -> R<&Ready> {
        let wanted = self.settings_for(config)?;
        let ready = self
            .ready
            .get_or_try_init(|| make_ready(wanted, &self.root))
            .await
            .map_err(|e| PartError::internal(format!("{e:#}")))?;
        if ready.stack.settings.conflicts_with(config) {
            return Err(PartError::invalid(
                "this program already runs another twenty-crm server",
            ));
        }
        Ok(ready)
    }

    /// `self.settings` with what a running instance's record says, then `config` over that, so a
    /// restarted program finds the server its instances are on.
    fn settings_for(&self, config: &Value) -> R<Settings> {
        let invalid = |e: anyhow::Error| PartError::invalid(format!("{e:#}"));
        let known = instances::recorded_settings(&self.root);
        let start = known.as_ref().unwrap_or(&self.settings);
        start.with_config(config).map_err(invalid)
    }

    fn rest(&self, ready: &Ready, instance: &str) -> R<Rest> {
        let port = instances::port_of(&self.root, instance)?;
        Rest::new(port, &ready.creds.api_key).map_err(PartError::internal_from)
    }
}

async fn make_ready(settings: Settings, root: &std::path::Path) -> anyhow::Result<Ready> {
    let stack = Stack::new(settings)?;
    stack.up().await?;
    let creds = base::load_or_make(&stack, root).await?;
    Ok(Ready { stack, creds })
}

#[async_trait]
impl Part for TwentyCrm {
    async fn describe(&self) -> R<Describe> {
        Ok(Describe {
            name: "twenty-crm".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: false,
            fingerprint: json!({"image": self.settings.image}),
            tools: tool_schemas::schemas(),
        })
    }

    async fn start(&self, p: StartParams) -> R<()> {
        self.start_instance(p).await
    }

    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput> {
        let ready = self.ready(&Value::Null).await?;
        let rest = self.rest(ready, &p.instance)?;
        tools::call(&rest, &p.name, &p.args).await
    }

    async fn save(&self, p: SaveParams) -> R<Saved> {
        self.save_instance(p).await
    }

    async fn inspect(&self, p: InspectParams) -> R<Docs> {
        self.inspect_target(p).await
    }

    async fn diff(&self, p: DiffParams) -> R<Differing> {
        self.diff_targets(p).await
    }

    async fn drop_save(&self, p: SaveId) -> R<()> {
        self.drop_save_db(&p.save).await
    }

    async fn stop(&self, p: IdParams) -> R<()> {
        self.stop_instance(&p.instance).await
    }

    async fn usage(&self) -> R<Usage> {
        self.bytes_used().await
    }

    async fn gc(&self, p: GcParams) -> R<GcResult> {
        self.collect(&p.keep).await
    }

    async fn connect(&self, p: IdParams) -> R<Value> {
        let ready = self.ready(&Value::Null).await?;
        let port = instances::port_of(&self.root, &p.instance)?;
        let c = &ready.creds;
        Ok(
            json!({"url": format!("http://127.0.0.1:{port}"), "api_key": c.api_key,
                  "admin_email": c.admin_email, "admin_password": c.admin_password}),
        )
    }
}
