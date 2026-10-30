use std::{collections::HashMap, sync::Mutex};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::sync::{broadcast, Mutex as AsyncMutex};
use tokio_postgres::{Client, NoTls, Row};
use world::pg::Admin;

use crate::model::*;

pub type RunId = String;
pub type BranchId = String;

pub const DEFAULT_DB: &str = "arena";

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS runs (
    id text PRIMARY KEY,
    task_id text NOT NULL,
    team jsonb NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE IF NOT EXISTS branches (
    id text PRIMARY KEY,
    run_id text NOT NULL REFERENCES runs(id),
    parent_id text REFERENCES branches(id),
    start_seq bigint,
    team jsonb NOT NULL,
    changes jsonb NOT NULL DEFAULT '[]',
    fork_note text,
    status text NOT NULL DEFAULT 'running',
    error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    ended_at timestamptz
);
CREATE TABLE IF NOT EXISTS events (
    branch_id text NOT NULL REFERENCES branches(id),
    seq bigint NOT NULL,
    at timestamptz NOT NULL DEFAULT clock_timestamp(),
    agent text,
    kind text NOT NULL,
    payload jsonb NOT NULL,
    PRIMARY KEY (branch_id, seq)
);
"#;

const BRANCH_COLS: &str =
    "id, run_id, parent_id, start_seq, team, changes, fork_note, status, error, created_at, ended_at";
const EVENT_COLS: &str = "branch_id, seq, at, agent, kind, payload";

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

pub struct Store {
    client: Client,
    tx: broadcast::Sender<Event>,
    // Per-branch locks make `max(seq)+1` appends atomic within this process.
    locks: Mutex<HashMap<String, std::sync::Arc<AsyncMutex<()>>>>,
}

impl Store {
    /// Opens (creating if needed) the `arena` metadata database.
    pub async fn open(admin: &Admin) -> Result<Store> {
        Store::open_named(admin, DEFAULT_DB).await
    }

    /// Like `open`, with a custom database name (tests use a fresh one each).
    pub async fn open_named(admin: &Admin, name: &str) -> Result<Store> {
        if !admin.exists(name).await? {
            // Concurrent creators may race; tolerate losing.
            if let Err(e) = admin.create_db_from(name, "template1").await {
                if !admin.exists(name).await? {
                    return Err(e).context("creating metadata database");
                }
            }
        }
        let (client, conn) = tokio_postgres::connect(&admin.db_url(name), NoTls)
            .await
            .with_context(|| format!("connecting to {name}"))?;
        tokio::spawn(conn);
        client.batch_execute(SCHEMA).await?;
        client
            .execute(
                "UPDATE branches SET status = 'interrupted', ended_at = now() WHERE status = 'running'",
                &[],
            )
            .await?;
        let (tx, _) = broadcast::channel(1024);
        Ok(Store { client, tx, locks: Mutex::new(HashMap::new()) })
    }

    fn lock_for(&self, branch: &str) -> std::sync::Arc<AsyncMutex<()>> {
        self.locks.lock().unwrap().entry(branch.to_string()).or_default().clone()
    }

    pub async fn create_run(&self, task_id: &str, team: &Team) -> Result<RunId> {
        let id = new_id();
        self.client
            .execute(
                "INSERT INTO runs (id, task_id, team) VALUES ($1, $2, $3)",
                &[&id, &task_id, &serde_json::to_value(team)?],
            )
            .await?;
        Ok(id)
    }

    /// `team` is the branch's effective team (the run's team with `changes` applied).
    pub async fn create_branch(
        &self,
        run_id: &str,
        parent: Option<(&str, i64)>,
        team: &Team,
        changes: &Value,
        fork_note: Option<String>,
    ) -> Result<BranchId> {
        let id = new_id();
        let (parent_id, start_seq) = match parent {
            Some((p, s)) => (Some(p.to_string()), Some(s)),
            None => (None, None),
        };
        self.client
            .execute(
                "INSERT INTO branches (id, run_id, parent_id, start_seq, team, changes, fork_note)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[&id, &run_id, &parent_id, &start_seq, &serde_json::to_value(team)?, changes, &fork_note],
            )
            .await?;
        Ok(id)
    }

    pub async fn append(
        &self,
        branch_id: &str,
        agent: Option<&str>,
        kind: EventKind,
        payload: Value,
    ) -> Result<Event> {
        let lock = self.lock_for(branch_id);
        let _g = lock.lock().await;
        let row = self
            .client
            .query_one(
                &format!(
                    "INSERT INTO events (branch_id, seq, agent, kind, payload)
                     SELECT $1, coalesce(max(seq), 0) + 1, $2, $3, $4 FROM events WHERE branch_id = $1
                     RETURNING {EVENT_COLS}"
                ),
                &[&branch_id, &agent, &kind.as_str(), &payload],
            )
            .await?;
        let ev = event_from(&row)?;
        let _ = self.tx.send(ev.clone()); // no subscribers is fine
        Ok(ev)
    }

    pub async fn events(&self, branch_id: &str) -> Result<Vec<Event>> {
        let rows = self
            .client
            .query(
                &format!("SELECT {EVENT_COLS} FROM events WHERE branch_id = $1 ORDER BY seq"),
                &[&branch_id],
            )
            .await?;
        rows.iter().map(event_from).collect()
    }

    pub async fn set_status(
        &self,
        branch_id: &str,
        status: BranchStatus,
        error: Option<String>,
    ) -> Result<()> {
        self.client
            .execute(
                "UPDATE branches SET status = $2, error = $3,
                 ended_at = CASE WHEN $2 = 'running' THEN NULL ELSE now() END WHERE id = $1",
                &[&branch_id, &status.as_str(), &error],
            )
            .await?;
        Ok(())
    }

    pub async fn branch(&self, id: &str) -> Result<Branch> {
        self.find_branch(id).await?.ok_or_else(|| anyhow!("no such branch: {id}"))
    }

    /// Like `branch`, but a missing branch is `Ok(None)` rather than an error.
    pub async fn find_branch(&self, id: &str) -> Result<Option<Branch>> {
        let row = self
            .client
            .query_opt(&format!("SELECT {BRANCH_COLS} FROM branches WHERE id = $1"), &[&id])
            .await?;
        row.as_ref().map(branch_from).transpose()
    }

    pub async fn runs(&self) -> Result<Vec<RunSummary>> {
        let runs = self
            .client
            .query("SELECT id, task_id, created_at FROM runs ORDER BY created_at DESC, id", &[])
            .await?;
        let branches = self
            .client
            .query(&format!("SELECT {BRANCH_COLS} FROM branches ORDER BY created_at, id"), &[])
            .await?;
        let mut by_run: HashMap<String, Vec<Branch>> = HashMap::new();
        for r in &branches {
            let b = branch_from(r)?;
            by_run.entry(b.run_id.clone()).or_default().push(b);
        }
        Ok(runs
            .iter()
            .map(|r| {
                let id: String = r.get("id");
                RunSummary {
                    branches: by_run.remove(&id).unwrap_or_default(),
                    id,
                    task_id: r.get("task_id"),
                    created_at: r.get("created_at"),
                }
            })
            .collect())
    }

    /// Every database the store refers to: `branch_<id>` for each branch and each snapshot
    /// named by a `ToolCall` event.
    pub async fn referenced_dbs(&self) -> Result<std::collections::HashSet<String>> {
        let mut out = std::collections::HashSet::new();
        for r in self.client.query("SELECT id FROM branches", &[]).await? {
            out.insert(format!("branch_{}", r.get::<_, String>(0)));
        }
        out.extend(self.snapshots_where("TRUE", &[]).await?);
        Ok(out)
    }

    async fn snapshots_where(&self, cond: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> Result<Vec<String>> {
        let sql = format!(
            "SELECT DISTINCT payload->>'snapshot' FROM events
             WHERE kind = 'tool_call' AND payload->>'snapshot' IS NOT NULL AND {cond}"
        );
        Ok(self.client.query(&sql, params).await?.iter().map(|r| r.get(0)).collect())
    }

    /// Deletes the run's branches, events and the run itself, and returns every database
    /// they referenced (`branch_<id>` and the snapshots its events name).
    pub async fn delete_run(&self, run_id: &str) -> Result<Vec<String>> {
        let ids: Vec<String> = self
            .client
            .query("SELECT id FROM branches WHERE run_id = $1", &[&run_id])
            .await?
            .iter()
            .map(|r| r.get(0))
            .collect();
        let mut dbs: Vec<String> = ids.iter().map(|i| format!("branch_{i}")).collect();
        dbs.extend(self.snapshots_where("branch_id = ANY($1)", &[&ids]).await?);
        dbs.sort();
        dbs.dedup();
        // One statement, so the branch/parent foreign keys are checked at its end.
        self.client
            .execute(
                "WITH ev AS (DELETE FROM events WHERE branch_id = ANY($2)),
                      br AS (DELETE FROM branches WHERE run_id = $1)
                 DELETE FROM runs WHERE id = $1",
                &[&run_id, &ids],
            )
            .await?;
        Ok(dbs)
    }

    /// True if some branch of the run has status `running`.
    pub async fn run_has_running(&self, run_id: &str) -> Result<bool> {
        Ok(self
            .client
            .query_one("SELECT EXISTS (SELECT 1 FROM branches WHERE run_id = $1 AND status = 'running')", &[&run_id])
            .await?
            .get(0))
    }

    pub async fn run_exists(&self, run_id: &str) -> Result<bool> {
        Ok(self.client.query_one("SELECT EXISTS (SELECT 1 FROM runs WHERE id = $1)", &[&run_id]).await?.get(0))
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    /// Copies `from`'s events with `seq < before_seq` onto `to`, keeping their seqs.
    pub async fn copy_prefix(&self, from_branch: &str, to_branch: &str, before_seq: i64) -> Result<()> {
        let lock = self.lock_for(to_branch);
        let _g = lock.lock().await;
        self.client
            .execute(
                "INSERT INTO events (branch_id, seq, at, agent, kind, payload)
                 SELECT $2, seq, at, agent, kind, payload FROM events
                 WHERE branch_id = $1 AND seq < $3",
                &[&from_branch, &to_branch, &before_seq],
            )
            .await?;
        Ok(())
    }
}

fn event_from(r: &Row) -> Result<Event> {
    let kind: String = r.get("kind");
    Ok(Event {
        branch_id: r.get("branch_id"),
        seq: r.get("seq"),
        at: r.get("at"),
        agent: r.get("agent"),
        kind: EventKind::parse(&kind).ok_or_else(|| anyhow!("unknown event kind {kind}"))?,
        payload: r.get("payload"),
    })
}

fn branch_from(r: &Row) -> Result<Branch> {
    let status: String = r.get("status");
    let team: Value = r.get("team");
    Ok(Branch {
        id: r.get("id"),
        run_id: r.get("run_id"),
        parent_id: r.get("parent_id"),
        start_seq: r.get("start_seq"),
        team: serde_json::from_value(team)?,
        changes: r.get("changes"),
        fork_note: r.get("fork_note"),
        status: BranchStatus::parse(&status).ok_or_else(|| anyhow!("unknown status {status}"))?,
        error: r.get("error"),
        created_at: r.get("created_at"),
        ended_at: r.get("ended_at"),
    })
}
