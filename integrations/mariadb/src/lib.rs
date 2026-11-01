//! The `mariadb` integration: each instance is a database `mi_<instance>` on a dedicated MariaDB
//! server with row-based binary logging, and each save records how to rebuild it: an optional
//! dump plus binlog segments (see `arena_part::helpers::mariadb`). Saves restore exactly.
mod disk;
mod instances;
mod keys;
mod rows;
mod saves;
mod servers;
mod settings;
mod sql;
mod views;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;

use arena_part::helpers::mariadb::{Blobs, IMAGE};
use arena_part::helpers::save_dirs::SaveDirs;
use arena_part::proto::*;
use arena_part::{Part, PartError};
use async_trait::async_trait;
use serde_json::{json, Value};

pub use keys::keys;
pub use settings::{Settings, DEFAULT_DUMP_AFTER};

use instances::Instance;
use servers::Servers;

pub struct Mariadb {
    dirs: SaveDirs,
    blobs: Blobs,
    /// `arena-mariadb --sql-tools`: list and serve `sql_query` and `sql_execute`.
    sql_tools: bool,
    servers: Servers,
    live: Mutex<HashMap<String, Instance>>,
    /// Scratch databases being read for `inspect` and `diff`.
    views: Mutex<HashSet<String>>,
    /// Held while a dump is made and while unused dumps are removed.
    blob_gate: tokio::sync::Mutex<()>,
}

type R<T> = Result<T, PartError>;

impl Mariadb {
    /// Saves under `$ARENA_SAVES/mariadb`.
    pub fn from_env() -> Mariadb {
        let dirs = SaveDirs::new("mariadb");
        Mariadb::new(dirs.root().to_path_buf())
    }

    /// Saves under `package_folder/<save>`, dumps under `package_folder/.blobs`.
    pub fn new(package_folder: PathBuf) -> Mariadb {
        let dirs = SaveDirs::under(package_folder);
        dirs.sweep_scratch();
        Mariadb {
            blobs: Blobs::new(dirs.root().join(".blobs")),
            dirs,
            sql_tools: false,
            servers: Servers::default(),
            live: Mutex::new(HashMap::new()),
            views: Mutex::new(HashSet::new()),
            blob_gate: tokio::sync::Mutex::new(()),
        }
    }

    /// Turns on the `sql_query` and `sql_execute` tools.
    pub fn with_sql_tools(mut self) -> Mariadb {
        self.sql_tools = true;
        self
    }
}

/// The server version every server of this part runs: the image's tag.
fn server_version() -> &'static str {
    IMAGE.rsplit(':').next().unwrap_or(IMAGE)
}

#[async_trait]
impl Part for Mariadb {
    async fn describe(&self) -> R<Describe> {
        Ok(Describe {
            name: "mariadb".into(),
            version: "0.1.0".into(),
            protocol: PROTOCOL,
            restore: Restore {
                exact: ExactKind::Save,
                approximate: false,
            },
            uses_clock: false,
            fingerprint: json!({"server_version": server_version()}),
            tools: if self.sql_tools { sql::tools() } else { vec![] },
        })
    }

    async fn start(&self, p: StartParams) -> R<()> {
        self.start_instance(p).await
    }

    async fn call_tool(&self, p: CallToolParams) -> R<ToolOutput> {
        self.tool(p).await
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
        let i = self.instance(&p.instance)?;
        let c = i.server.config();
        Ok(json!({
            "url": format!("mysql://root:{}@127.0.0.1:{}/{}", c.root_password, c.port, i.db),
        }))
    }
}
