//! Databases, accounts and the server's identity. Everything here runs as root, unlogged.
use anyhow::{bail, Context, Result};
use mysql_async::prelude::Queryable;

use super::login::{Login, DB_GRANTS};
use super::manifest::ServerRef;
use super::server::{check_account, check_db, Server};
use super::sql::{like_prefix, quote_ident};

/// The database holding the server's identity, made when the server is first ensured.
const META_DB: &str = "arena_meta";

/// `CREATE TABLE IF NOT EXISTS` for the one-row identity table.
fn server_table() -> String {
    format!(
        "CREATE TABLE IF NOT EXISTS {}.server (one TINYINT NOT NULL PRIMARY KEY \
         CHECK (one = 1), id CHAR(36) NOT NULL)",
        quote_ident(META_DB)
    )
}

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
    /// volume behind the same name) from the one it was made on. The table holds one row (key
    /// `one = 1`), so two ensures at once can't make two identities.
    pub(super) async fn ensure_identity(&self) -> Result<()> {
        let db = quote_ident(META_DB);
        self.migrate_identity().await?;
        self.unlogged(&format!(
            "CREATE DATABASE IF NOT EXISTS {db}; {}; \
             INSERT IGNORE INTO {db}.server (one, id) VALUES (1, UUID())",
            server_table()
        ))
        .await
    }

    /// An identity table from before the one-row key (`server (id)`) is made again with the
    /// key, keeping its first id (a table without one gets a new id from `ensure_identity`).
    async fn migrate_identity(&self) -> Result<()> {
        let Some(kept) = self.old_identity().await? else {
            return Ok(());
        };
        let db = quote_ident(META_DB);
        self.with_conn(async |c| {
            c.query_drop("SET sql_log_bin = 0").await?;
            c.query_drop(format!("DROP TABLE {db}.server; {}", server_table()))
                .await?;
            if let Some(id) = kept {
                let insert = format!("INSERT INTO {db}.server (one, id) VALUES (1, ?)");
                c.exec_drop(insert, (id,)).await?;
            }
            Ok(())
        })
        .await
    }

    /// `Some` when there is an identity table without the `one` column, with its first id.
    async fn old_identity(&self) -> Result<Option<Option<String>>> {
        let tables = "SELECT COUNT(*) FROM information_schema.TABLES \
                      WHERE TABLE_SCHEMA = ? AND TABLE_NAME = 'server'";
        let keyed = "SELECT COUNT(*) FROM information_schema.COLUMNS \
                     WHERE TABLE_SCHEMA = ? AND TABLE_NAME = 'server' AND COLUMN_NAME = 'one'";
        let first = format!("SELECT MIN(id) FROM {}.server", quote_ident(META_DB));
        self.with_conn(async |c| {
            let table: Option<u64> = c.exec_first(tables, (META_DB,)).await?;
            let keyed: Option<u64> = c.exec_first(keyed, (META_DB,)).await?;
            if table.unwrap_or(0) == 0 || keyed.unwrap_or(0) > 0 {
                return Ok(None);
            }
            let id: Option<Option<String>> = c.query_first(first).await?;
            Ok(Some(id.flatten()))
        })
        .await
    }

    /// The server's identity (see `ensure_identity`).
    pub async fn identity(&self) -> Result<String> {
        let sql = format!(
            "SELECT id FROM {}.server WHERE one = 1",
            quote_ident(META_DB)
        );
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
