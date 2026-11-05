//! The `erpnext` integration: one ERPNext install (compose project `arena_erpnext`) whose sites
//! are the instances. A site's database lives on the integration's MariaDB server, made by the
//! kit's mariadb helper with binary logging, so a save is that database's binlog manifest (see
//! `arena_part::helpers::mariadb`) plus a copy of the site's files. Saves restore exactly.
mod base;
mod disk;
mod doc_cache;
mod frappe;
mod instances;
mod jobs;
mod json_file;
mod names;
mod ready;
mod rest;
mod saves;
mod settings;
mod site_config;
mod sites;
mod stack;
mod tool_schemas;
mod tools;
mod views;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Mutex;

use arena_part::helpers::busy::BusySet;
use arena_part::helpers::mariadb::{Dumps, Server, DEFAULT_REROOT_AFTER};
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{OnceCell, RwLock};

pub use settings::{Settings, StackConfig, DEFAULT_DIFF_DOCTYPES, DEFAULT_DUMP_AFTER};

use instances::Instance;
use ready::Ready;

pub struct Erpnext {
    cfg: StackConfig,
    /// The server of `cfg`; once the install runs, use `server()`, which gives its server.
    default_server: Server,
    dirs: SaveDirs,
    /// The stack up and the base loaded (or made), on first need.
    ready: OnceCell<Ready>,
    live: Mutex<HashMap<String, Instance>>,
    /// Scratch sites being read for `inspect` and `diff`, by database name.
    busy: BusySet,
    /// Documents read from saves by `diff`, for the `inspect` that follows it.
    docs: doc_cache::DocCache,
    /// Dumps, and the gate held while a dump is made until a manifest names it and while
    /// unused dumps are removed.
    dumps: Dumps,
    /// Held (shared) while a site is built until it is live; taken alone to rotate the binlog.
    starts: RwLock<()>,
    versions: OnceCell<BTreeMap<String, String>>,
    /// Binlog bytes that saves kept by a `gc` pattern may hold before they are rewritten as dumps.
    reroot_after: u64,
}

type R<T> = Result<T, PartError>;

impl Erpnext {
    /// The install of env `ARENA_ERPNEXT_*`, saves under `$ARENA_SAVES/erpnext`.
    pub fn from_env() -> R<Erpnext> {
        let dirs = SaveDirs::new("erpnext");
        Ok(Erpnext::new(
            StackConfig::from_env()?,
            dirs.root().to_path_buf(),
        ))
    }

    /// Sets how many binlog bytes saves kept by a `gc` pattern may hold before they are re-rooted.
    pub fn with_reroot_after(mut self, bytes: u64) -> Erpnext {
        self.reroot_after = bytes;
        self
    }

    /// The install of `cfg`, with saves under `package_folder/<save>`, dumps in `.blobs`, the
    /// base in `.base` and the live sites in `.sites`.
    pub fn new(cfg: StackConfig, package_folder: PathBuf) -> Erpnext {
        let dirs = SaveDirs::under(package_folder);
        dirs.sweep_scratch();
        Erpnext {
            default_server: Server::new(cfg.server.clone()).expect("StackConfig names are checked"),
            dumps: Dumps::new(dirs.root().join(".blobs")),
            dirs,
            cfg,
            ready: OnceCell::new(),
            live: Mutex::new(HashMap::new()),
            busy: BusySet::default(),
            docs: doc_cache::DocCache::default(),
            starts: RwLock::new(()),
            reroot_after: DEFAULT_REROOT_AFTER,
            versions: OnceCell::new(),
        }
    }

    /// The image tag and the frappe and erpnext versions in it (`bench version`, once).
    async fn fingerprint(&self) -> R<Value> {
        let image = self.cfg.image.clone();
        let versions = self
            .versions
            .get_or_try_init(|| stack::image_versions(image))
            .await?;
        Ok(
            json!({"image": self.cfg.image_tag(), "frappe": versions["frappe"],
                  "erpnext": versions["erpnext"]}),
        )
    }
}

#[async_trait]
impl Part for Erpnext {
    async fn describe(&self) -> R<Describe> {
        Ok(Describe {
            name: "erpnext".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: false,
            fingerprint: self.fingerprint().await?,
            tools: tool_schemas::schemas(),
        })
    }

    async fn start(&self, p: StartParams) -> R<()> {
        self.start_instance(p).await
    }

    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput> {
        let site = self.instance(&p.instance)?.site.name;
        let ready = self.ready().await?;
        tools::call(&ready.rest, &site, &p.name, &p.args).await
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
        self.drop_save_files(&p.save).await
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
        let site = self.instance(&p.instance)?.site.name;
        let ready = self.ready().await?;
        let c = &ready.base.creds;
        Ok(
            json!({"url": format!("http://127.0.0.1:{}", ready.stack.cfg.port), "site": site,
                  "api_key": c.key.api_key, "api_secret": c.key.api_secret,
                  "admin_password": c.admin_password}),
        )
    }
}
