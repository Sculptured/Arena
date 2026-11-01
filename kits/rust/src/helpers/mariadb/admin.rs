//! Databases, accounts and the server's identity. Everything here runs as root, unlogged.
use anyhow::{bail, Context, Result};
use mysql_async::prelude::Queryable;

use super::login::{Login, DB_GRANTS};
use super::manifest::ServerRef;
use super::server::{check_account, check_db, Server};
use super::sql::{like_prefix, quote_ident};

/// The database holding the server's identity, made when the server is first ensured.
const META_DB: &str = "arena_meta";

impl Server {
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
        let pattern = like_prefix(prefix);
        self.with_conn(async |c| Ok(c.exec(sql, (pattern,)).await?))
            .await
    }

    pub async fn exists(&self, db: &str) -> Result<bool> {
        let sql = "SELECT COUNT(*) FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?";
        let n: Option<u64> = self
            .with_conn(async |c| Ok(c.exec_first(sql, (db,)).await?))
            .await?;
        Ok(n.unwrap_or(0) > 0)
    }

    /// A new account `user` (an old one is dropped first) with a random password, that may use
    /// only database `db` ([`DB_GRANTS`]), plus the global privileges `extra` (may be empty).
    pub async fn create_login(&self, user: &str, db: &str, extra: &str) -> Result<Login> {
        check_db(db)?;
        self.drop_login(user).await?;
        let login = Login::new_random(user)?;
        let who = format!("{}@'%'", quote_ident(user));
        let mut sql = format!(
            "CREATE USER {who} IDENTIFIED BY '{}'; GRANT {DB_GRANTS} ON {}.* TO {who};",
            login.password,
            quote_ident(db)
        );
        if !extra.is_empty() {
            sql.push_str(&format!(" GRANT {extra} ON *.* TO {who};"));
        }
        self.unlogged(&sql).await?;
        Ok(login)
    }

    pub async fn drop_login(&self, user: &str) -> Result<()> {
        check_account(user)?;
        self.unlogged(&format!("DROP USER IF EXISTS {}@'%'", quote_ident(user)))
            .await
    }

    /// The account names that start with `prefix`.
    pub async fn logins(&self, prefix: &str) -> Result<Vec<String>> {
        let sql = "SELECT DISTINCT User FROM mysql.user WHERE User LIKE ? ORDER BY 1";
        let pattern = like_prefix(prefix);
        self.with_conn(async |c| Ok(c.exec(sql, (pattern,)).await?))
            .await
    }

    /// Gives the server a random identity once, so a save can tell a replaced server (a new
    /// volume behind the same name) from the one it was made on.
    pub(super) async fn ensure_identity(&self) -> Result<()> {
        let db = quote_ident(META_DB);
        self.unlogged(&format!(
            "CREATE DATABASE IF NOT EXISTS {db}; \
             CREATE TABLE IF NOT EXISTS {db}.server (id CHAR(36) NOT NULL); \
             INSERT INTO {db}.server SELECT UUID() FROM DUAL \
             WHERE NOT EXISTS (SELECT 1 FROM {db}.server)"
        ))
        .await
    }

    /// The server's identity (see `ensure_identity`).
    pub async fn identity(&self) -> Result<String> {
        let sql = format!("SELECT id FROM {}.server LIMIT 1", quote_ident(META_DB));
        let id: Option<String> = self
            .with_conn(async |c| Ok(c.query_first(sql).await?))
            .await
            .context("reading the server's identity")?;
        match id {
            Some(id) => Ok(id),
            None => bail!("the server in {} has no identity", self.config().container),
        }
    }

    /// What a manifest records about this server.
    pub async fn reference(&self) -> Result<ServerRef> {
        Ok(ServerRef {
            container: self.config().container.clone(),
            port: self.config().port,
            id: self.identity().await?,
        })
    }

    /// The server's version (`11.8.3-MariaDB-..`).
    pub async fn version(&self) -> Result<String> {
        let v: Option<String> = self
            .with_conn(async |c| Ok(c.query_first("SELECT VERSION()").await?))
            .await?;
        v.context("SELECT VERSION() gave nothing")
    }
}
