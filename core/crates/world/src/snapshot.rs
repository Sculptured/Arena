//! Branches and snapshots: every database copy goes through one per-database lock.

use std::{collections::HashMap, sync::Arc, sync::Mutex};

use anyhow::{anyhow, Result};
use serde_json::Value;
use tokio_postgres::Client;

use crate::{gold::ensure_gold, pg::Admin, schema::BASE_DB};

/// Where a new branch is copied from.
pub enum Source {
    Base,
    Snapshot(String),
    /// A task JSON; its `gold_<id>` database is created on demand.
    Gold(Value),
}

/// Single gate for connections to, and copies from, branch/snapshot/gold databases.
/// Postgres refuses `CREATE DATABASE ... TEMPLATE x` while anyone is connected to `x`,
/// so one lock per database name serializes copies against connections.
pub struct Worlds {
    admin: Admin,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

fn check_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(anyhow!("invalid database name {name:?}"));
    }
    Ok(())
}

impl Worlds {
    pub fn new(admin: Admin) -> Worlds {
        Worlds {
            admin,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn admin(&self) -> &Admin {
        &self.admin
    }

    fn lock_for(&self, db: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(db.to_string())
            .or_default()
            .clone()
    }

    /// Creates `branch_<id>` from `from` and returns its name.
    pub async fn new_branch(&self, id: &str, from: Source) -> Result<String> {
        let name = format!("branch_{id}");
        check_name(&name)?;
        let src = match from {
            Source::Base => BASE_DB.to_string(),
            Source::Snapshot(db) => db,
            Source::Gold(task) => {
                let _g = self.lock_for(BASE_DB).lock_owned().await;
                ensure_gold(&self.admin, &task).await?
            }
        };
        check_name(&src)?;
        let _g = self.lock_for(&src).lock_owned().await;
        self.admin.create_db_from(&name, &src).await?;
        Ok(name)
    }

    /// Copies `branch_db` to a fresh `snap_<uuid>` and returns its name.
    pub async fn snapshot(&self, branch_db: &str) -> Result<String> {
        check_name(branch_db)?;
        let name = format!("snap_{}", uuid::Uuid::new_v4().simple());
        let _g = self.lock_for(branch_db).lock_owned().await;
        self.admin.create_db_from(&name, branch_db).await?;
        Ok(name)
    }

    /// Holds `db`'s lock, connects, runs `f`, and closes the connection before releasing.
    pub async fn with_client<T>(
        &self,
        db: &str,
        f: impl AsyncFnOnce(&mut Client) -> Result<T>,
    ) -> Result<T> {
        check_name(db)?;
        let _g = self.lock_for(db).lock_owned().await;
        self.admin.with_db(db, f).await
    }

    /// Same as `drop_db`; note `arc.drop(..)` resolves to `Drop::drop` on an `Arc<Worlds>`,
    /// so Arc holders should call `drop_db`.
    pub async fn drop(&self, db: &str) -> Result<()> {
        self.drop_db(db).await
    }

    pub async fn drop_db(&self, db: &str) -> Result<()> {
        check_name(db)?;
        let _g = self.lock_for(db).lock_owned().await;
        self.admin.drop_db(db).await
    }
}
