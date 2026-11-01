//! One MariaDB server in a container: start it and connect to it. Administration, the binlog
//! and dumps are in `admin.rs`, `binlog.rs` and `dumps.rs`.
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder};

use super::docker;
use super::login::Login;

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

/// A name of `[a-z0-9_]`, 1 to `max` characters.
fn check_name(what: &str, name: &str, max: usize) -> Result<()> {
    let ok = !name.is_empty()
        && name.len() <= max
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !ok {
        bail!("{name:?} is not a {what} name this part uses");
    }
    Ok(())
}

/// A database name this helper creates or drops: `[a-z0-9_]{1,64}` (MariaDB's limit).
pub(super) fn check_db(name: &str) -> Result<()> {
    check_name("database", name, 64)
}

/// An account name: `[a-z0-9_]{1,80}` (a replay account is `rp_` and a database name).
pub(super) fn check_account(name: &str) -> Result<()> {
    check_name("account", name, 80)
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

    /// Starts the container if it is not running, waits until the server takes connections,
    /// and gives a new server its identity.
    pub async fn ensure(&self) -> Result<()> {
        let cfg = self.config.clone();
        tokio::task::spawn_blocking(move || docker::ensure_container(&cfg)).await??;
        self.wait_ready().await?;
        self.ensure_identity().await
    }

    /// Whether the server takes connections now.
    pub async fn reachable(&self) -> bool {
        self.with_conn(async |_| Ok(())).await.is_ok()
    }

    async fn wait_ready(&self) -> Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        loop {
            match self.with_conn(async |_| Ok(())).await {
                Ok(()) => return Ok(()),
                Err(e) if Instant::now() > deadline => {
                    return Err(e.context(format!("{} did not come up", self.config.container)))
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
    }

    fn opts(&self, login: &Login, db: Option<&str>) -> Opts {
        OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(self.config.port)
            .prefer_socket(false)
            .user(Some(login.user.clone()))
            .pass(Some(login.password.clone()))
            .db_name(db)
            .into()
    }

    /// Connects as `login`, with `db` as the default database when given, runs `f`, and closes
    /// the connection.
    pub async fn with_login<T>(
        &self,
        login: &Login,
        db: Option<&str>,
        f: impl AsyncFnOnce(&mut Conn) -> Result<T>,
    ) -> Result<T> {
        let c = Conn::new(self.opts(login, db)).await;
        let mut c =
            c.with_context(|| format!("connecting to MariaDB in {}", self.config.container))?;
        let out = f(&mut c).await;
        let _ = c.disconnect().await;
        out
    }

    /// Runs `f` as root with no default database.
    pub async fn with_conn<T>(&self, f: impl AsyncFnOnce(&mut Conn) -> Result<T>) -> Result<T> {
        self.with_login(&Login::root(&self.config), None, f).await
    }

    /// Runs `f` as root with `db` as the default database.
    pub async fn with_db<T>(
        &self,
        db: &str,
        f: impl AsyncFnOnce(&mut Conn) -> Result<T>,
    ) -> Result<T> {
        self.with_login(&Login::root(&self.config), Some(db), f)
            .await
    }

    /// Runs `sql` as root on a connection whose statements stay out of the binlog.
    pub(super) async fn unlogged(&self, sql: &str) -> Result<()> {
        self.with_conn(async |c| {
            c.query_drop("SET sql_log_bin = 0").await?;
            Ok(c.query_drop(sql).await?)
        })
        .await
    }
}
