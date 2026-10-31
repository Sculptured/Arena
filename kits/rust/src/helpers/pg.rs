//! Postgres databases as templates: copy, drop, size and list databases on one server.
//!
//! Postgres refuses `CREATE DATABASE .. TEMPLATE x` while anyone is connected to `x`, so there is one
//! lock per database, and [`Templates::with_db`] closes its connection before it lets go of the lock.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
pub use tokio_postgres;
use tokio_postgres::{Client, NoTls};

/// Copies, drops and lists databases on a Postgres server.
pub struct Templates {
    base_url: String,
    admin: tokio::sync::Mutex<Option<Client>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(anyhow!("invalid database name {name:?}"));
    }
    Ok(())
}

fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

/// `CREATE DATABASE new TEMPLATE template`, after checking both names.
fn create_from_sql(new: &str, template: &str) -> Result<String> {
    check_name(new)?;
    check_name(template)?;
    Ok(format!(
        "CREATE DATABASE {} TEMPLATE {}",
        quote(new),
        quote(template)
    ))
}

/// How many times `create_from_exclusive` retries while terminated backends go away.
const CREATE_ATTEMPTS: u32 = 40;

async fn terminate_others(c: &Client, db: &str) -> Result<()> {
    c.execute(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = $1 AND pid <> pg_backend_pid()",
        &[&db],
    )
    .await?;
    Ok(())
}

fn is_object_in_use(e: &tokio_postgres::Error) -> bool {
    e.as_db_error()
        .is_some_and(|d| d.code() == &tokio_postgres::error::SqlState::OBJECT_IN_USE)
}

fn strip_db(url: &str) -> String {
    let url = url.trim_end_matches('/');
    match url.find("://") {
        Some(i) => match url[i + 3..].find('/') {
            Some(j) => url[..i + 3 + j].to_string(),
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}

impl Templates {
    /// `admin_url` is a server URL such as `postgres://arena:arena@127.0.0.1:5544/postgres`; a
    /// trailing `/dbname` is ignored. Connects on first use.
    pub fn new(admin_url: &str) -> Templates {
        Templates {
            base_url: strip_db(admin_url),
            admin: tokio::sync::Mutex::new(None),
            locks: Mutex::new(HashMap::new()),
        }
    }

    /// Connection URL of a database on this server.
    pub fn db_url(&self, db: &str) -> String {
        format!("{}/{}", self.base_url, db)
    }

    fn lock_for(&self, db: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(db.to_string())
            .or_default()
            .clone()
    }

    /// Runs `f` on the admin connection (reconnecting if it dropped).
    async fn admin<T>(&self, f: impl AsyncFnOnce(&Client) -> Result<T>) -> Result<T> {
        let mut g = self.admin.lock().await;
        if g.as_ref().is_none_or(|c| c.is_closed()) {
            let (client, conn) =
                tokio_postgres::connect(&format!("{}/postgres", self.base_url), NoTls)
                    .await
                    .with_context(|| format!("connecting to {}", self.base_url))?;
            tokio::spawn(conn);
            *g = Some(client);
        }
        f(g.as_ref().unwrap()).await
    }

    /// `CREATE DATABASE new TEMPLATE template`, holding the template's lock.
    pub async fn create_from(&self, new: &str, template: &str) -> Result<()> {
        let sql = create_from_sql(new, template)?;
        let _g = self.lock_for(template).lock_owned().await;
        self.admin(async |c| Ok(c.batch_execute(&sql).await?)).await
    }

    /// Like [`create_from`](Self::create_from), but first terminates every other connection to
    /// `template` (someone holding their own connection would make the copy fail), under the
    /// template's lock.
    pub async fn create_from_exclusive(&self, new: &str, template: &str) -> Result<()> {
        let sql = create_from_sql(new, template)?;
        let _g = self.lock_for(template).lock_owned().await;
        self.admin(async |c| {
            for attempt in 0..CREATE_ATTEMPTS {
                terminate_others(c, template).await?;
                match c.batch_execute(&sql).await {
                    Ok(()) => return Ok(()),
                    // the terminated backends take a moment to go
                    Err(e) if attempt + 1 < CREATE_ATTEMPTS && is_object_in_use(&e) => {
                        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            unreachable!("the last attempt returns")
        })
        .await
    }

    /// `CREATE DATABASE name` (empty).
    pub async fn create_empty(&self, name: &str) -> Result<()> {
        check_name(name)?;
        let sql = format!("CREATE DATABASE {}", quote(name));
        self.admin(async |c| Ok(c.batch_execute(&sql).await?)).await
    }

    /// Drops a database, even with connections open. Not an error if it is gone.
    pub async fn drop(&self, name: &str) -> Result<()> {
        check_name(name)?;
        let _g = self.lock_for(name).lock_owned().await;
        let sql = format!("DROP DATABASE IF EXISTS {} WITH (FORCE)", quote(name));
        self.admin(async |c| Ok(c.batch_execute(&sql).await?)).await
    }

    pub async fn exists(&self, name: &str) -> Result<bool> {
        self.admin(async |c| {
            let row = c
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)",
                    &[&name],
                )
                .await?;
            Ok(row.get(0))
        })
        .await
    }

    /// Size in bytes; `None` if the database is gone or is being dropped.
    pub async fn size(&self, name: &str) -> Result<Option<i64>> {
        self.admin(async |c| Ok(size_of(c, name).await)).await
    }

    /// Databases whose name starts with `prefix` (plain text, not a LIKE pattern) with their sizes,
    /// ordered by name. A database dropped mid-query, or being dropped, is skipped.
    pub async fn list(&self, prefix: &str) -> Result<Vec<(String, i64)>> {
        self.admin(async |c| {
            let rows = c
                .query(
                    "SELECT datname FROM pg_database \
                     WHERE left(datname, length($1::text)) = $1::text ORDER BY 1",
                    &[&prefix],
                )
                .await?;
            let mut out = vec![];
            for r in rows {
                let name: String = r.get(0);
                if let Some(size) = size_of(c, &name).await {
                    out.push((name, size));
                }
            }
            Ok(out)
        })
        .await
    }

    /// Holds `db`'s lock, connects, runs `f`, and closes the connection (for real) before it lets go
    /// of the lock, so a later copy of `db` finds nobody connected.
    pub async fn with_db<T>(
        &self,
        db: &str,
        f: impl AsyncFnOnce(&mut Client) -> Result<T>,
    ) -> Result<T> {
        check_name(db)?;
        let _g = self.lock_for(db).lock_owned().await;
        let (mut client, conn) = tokio_postgres::connect(&self.db_url(db), NoTls)
            .await
            .with_context(|| format!("connecting to {db}"))?;
        let handle = tokio::spawn(conn);
        let out = f(&mut client).await;
        drop(client);
        let _ = handle.await;
        out
    }
}

/// pg_database_size errors if the database vanished and gives NULL if it is being dropped.
async fn size_of(c: &Client, name: &str) -> Option<i64> {
    let row = c
        .query_one("SELECT pg_database_size($1::text)", &[&name])
        .await
        .ok()?;
    row.try_get::<_, Option<i64>>(0).ok().flatten()
}
