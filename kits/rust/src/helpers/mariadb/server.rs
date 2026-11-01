//! One MariaDB server in a container: start it, query it, dump, load and replay its binlog.
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, Row};

use super::blobs::Blobs;
use super::docker::{self, Replay};
use super::manifest::{Manifest, Segment, ServerRef};
use super::position::{span, BinlogFile, Position};
use super::sql::{like_prefix, quote_ident, valid_binlog_name};

/// The server image; it is never pulled, so it must be present locally.
pub const IMAGE: &str = "mariadb:11.8";
/// How long a newly started server may take to accept connections.
const READY_TIMEOUT: Duration = Duration::from_secs(120);

/// Which container runs the server, where it listens and where its data lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub container: String,
    pub port: u16,
    pub volume: String,
    pub root_password: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            container: "arena-mariadb".into(),
            port: 3307,
            volume: "arena_mariadb_data".into(),
            root_password: "arena".into(),
        }
    }
}

/// A container or volume name: `[A-Za-z0-9][A-Za-z0-9_.-]*`, at most 63 characters.
pub fn valid_docker_name(s: &str) -> bool {
    s.len() <= 63
        && s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// A database name this helper creates or drops: `[a-z0-9_]{1,64}`.
fn check_db(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !ok {
        bail!("{name:?} is not a database name this part uses");
    }
    Ok(())
}

/// A handle on one server. Creating it does nothing; [`Server::ensure`] starts the container.
#[derive(Debug, Clone)]
pub struct Server {
    config: ServerConfig,
}

impl Server {
    pub fn new(config: ServerConfig) -> Result<Server> {
        if !valid_docker_name(&config.container) || !valid_docker_name(&config.volume) {
            bail!("container and volume names must match [A-Za-z0-9][A-Za-z0-9_.-]{{0,62}}");
        }
        Ok(Server { config })
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// What a manifest records about this server.
    pub fn reference(&self) -> ServerRef {
        ServerRef {
            container: self.config.container.clone(),
            port: self.config.port,
        }
    }

    /// Starts the container if it is not running, then waits until the server takes connections.
    pub async fn ensure(&self) -> Result<()> {
        let cfg = self.config.clone();
        tokio::task::spawn_blocking(move || docker::ensure_container(&cfg)).await??;
        self.wait_ready().await
    }

    async fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            match self.conn(None).await {
                Ok(c) => return Ok(c.disconnect().await?),
                Err(e) if Instant::now() > deadline => {
                    return Err(e.context(format!("{} did not come up", self.config.container)))
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    fn opts(&self, db: Option<&str>) -> Opts {
        OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(self.config.port)
            .prefer_socket(false)
            .user(Some("root"))
            .pass(Some(self.config.root_password.clone()))
            .db_name(db)
            .into()
    }

    /// A connection, with `db` as the default database when given.
    pub async fn conn(&self, db: Option<&str>) -> Result<Conn> {
        let c = Conn::new(self.opts(db)).await;
        c.with_context(|| format!("connecting to MariaDB in {}", self.config.container))
    }

    /// Connects with `db` as the default database, runs `f`, and closes the connection.
    pub async fn with_db<T>(
        &self,
        db: &str,
        f: impl AsyncFnOnce(&mut Conn) -> Result<T>,
    ) -> Result<T> {
        let mut c = self.conn(Some(db)).await?;
        let out = f(&mut c).await;
        let _ = c.disconnect().await;
        out
    }

    /// Runs `sql` on a connection whose statements stay out of the binlog.
    async fn unlogged(&self, sql: &str) -> Result<()> {
        let mut c = self.conn(None).await?;
        c.query_drop("SET sql_log_bin = 0").await?;
        c.query_drop(sql).await?;
        Ok(c.disconnect().await?)
    }

    /// A new empty database `name` (an old one of that name is dropped first).
    pub async fn create_db(&self, name: &str) -> Result<()> {
        self.drop_db(name).await?;
        self.unlogged(&format!("CREATE DATABASE {}", quote_ident(name)))
            .await
    }

    pub async fn drop_db(&self, name: &str) -> Result<()> {
        check_db(name)?;
        self.unlogged(&format!("DROP DATABASE IF EXISTS {}", quote_ident(name)))
            .await
    }

    /// The databases whose names start with `prefix`, with their sizes in bytes.
    pub async fn databases(&self, prefix: &str) -> Result<Vec<(String, u64)>> {
        let sql = "SELECT s.SCHEMA_NAME, CAST(COALESCE(SUM(t.DATA_LENGTH + t.INDEX_LENGTH), 0) \
                   AS UNSIGNED) FROM information_schema.SCHEMATA s LEFT JOIN \
                   information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME \
                   WHERE s.SCHEMA_NAME LIKE ? GROUP BY s.SCHEMA_NAME ORDER BY 1";
        let mut c = self.conn(None).await?;
        let rows: Vec<(String, u64)> = c.exec(sql, (like_prefix(prefix),)).await?;
        c.disconnect().await?;
        Ok(rows)
    }

    pub async fn exists(&self, db: &str) -> Result<bool> {
        let mut c = self.conn(None).await?;
        let sql = "SELECT COUNT(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?";
        let n: Option<u64> = c.exec_first(sql, (db,)).await?;
        c.disconnect().await?;
        Ok(n.unwrap_or(0) > 0)
    }

    /// The end of the binlog now.
    pub async fn position(&self) -> Result<Position> {
        let mut c = self.conn(None).await?;
        let row: Option<Row> = c.query_first("SHOW BINLOG STATUS").await?;
        c.disconnect().await?;
        let row =
            row.ok_or_else(|| anyhow!("binary logging is off in {}", self.config.container))?;
        let file: String = row.get(0).context("SHOW BINLOG STATUS: no file")?;
        let pos: u64 = row.get(1).context("SHOW BINLOG STATUS: no position")?;
        Ok(Position::new(file, pos))
    }

    /// The binlog files on the server, oldest first.
    pub async fn binlog_files(&self) -> Result<Vec<BinlogFile>> {
        let mut c = self.conn(None).await?;
        let rows: Vec<Row> = c.query("SHOW BINARY LOGS").await?;
        c.disconnect().await?;
        rows.into_iter()
            .map(|r| {
                let name: Option<String> = r.get(0);
                let size: Option<u64> = r.get(1);
                Ok(BinlogFile::new(
                    name.context("no Log_name")?,
                    size.context("no File_size")?,
                ))
            })
            .collect()
    }

    /// Removes the binlog files before `file`.
    pub async fn purge_to(&self, file: &str) -> Result<()> {
        if !valid_binlog_name(file) {
            bail!("{file:?} is not a binlog file name");
        }
        let mut c = self.conn(None).await?;
        c.query_drop(format!("PURGE BINARY LOGS TO '{file}'"))
            .await?;
        Ok(c.disconnect().await?)
    }

    /// The server's version (`11.8.3-MariaDB-..`).
    pub async fn version(&self) -> Result<String> {
        let mut c = self.conn(None).await?;
        let v: Option<String> = c.query_first("SELECT VERSION()").await?;
        c.disconnect().await?;
        v.context("SELECT VERSION() gave nothing")
    }

    /// Dumps `db` into `blobs`; returns the blob id.
    pub async fn dump(&self, db: &str, blobs: &Blobs) -> Result<String> {
        check_db(db)?;
        let (cfg, db, blobs) = (self.config.clone(), db.to_string(), blobs.clone());
        let task = move || docker::dump(&cfg, &db, |r| blobs.write_from(r));
        tokio::task::spawn_blocking(task).await?
    }

    /// Runs the dump `id` from `blobs` in `db`, unlogged.
    pub async fn load(&self, blobs: &Blobs, id: &str, db: &str) -> Result<()> {
        check_db(db)?;
        let mut src = blobs.open(id).with_context(|| format!("dump {id}"))?;
        let (cfg, db) = (self.config.clone(), db.to_string());
        let task = move || docker::load(&cfg, &db, &mut src);
        tokio::task::spawn_blocking(task).await?
    }

    /// Replays `seg` (its database's events between its positions) into `into`, unlogged.
    pub async fn apply_log(&self, seg: &Segment, into: &str) -> Result<()> {
        check_db(&seg.db)?;
        check_db(into)?;
        let files = self.binlog_files().await?;
        let names = span(&files, &seg.from, &seg.to)?;
        let dir = self.binlog_dir().await?;
        let replay = Replay {
            db: seg.db.clone(),
            into: into.to_string(),
            files: names.iter().map(|f| format!("{dir}/{}", f.name)).collect(),
            start: seg.from.pos,
            stop: seg.to.pos,
        };
        let cfg = self.config.clone();
        let task = move || docker::apply_binlog(&cfg, &replay);
        tokio::task::spawn_blocking(task).await?
    }

    /// The folder of the binlog files inside the container.
    async fn binlog_dir(&self) -> Result<String> {
        let mut c = self.conn(None).await?;
        let base: Option<String> = c.query_first("SELECT @@log_bin_basename").await?;
        c.disconnect().await?;
        let base = base.context("binary logging is off")?;
        let (dir, _) = base.rsplit_once('/').context("odd log_bin_basename")?;
        Ok(dir.to_string())
    }

    /// Builds `target` from `m`: a new database, the root dump, then each segment. Returns the
    /// binlog position after, where `target`'s own changes begin. `target` is dropped on failure.
    pub async fn restore(&self, m: &Manifest, blobs: &Blobs, target: &str) -> Result<Position> {
        self.create_db(target).await?;
        let built = self.rebuild(m, blobs, target).await;
        if built.is_err() {
            let _ = self.drop_db(target).await;
        }
        built?;
        self.position().await
    }

    async fn rebuild(&self, m: &Manifest, blobs: &Blobs, target: &str) -> Result<()> {
        if let Some(root) = &m.root {
            self.load(blobs, root, target).await?;
        }
        for seg in &m.segments {
            self.apply_log(seg, target).await?;
        }
        Ok(())
    }
}
