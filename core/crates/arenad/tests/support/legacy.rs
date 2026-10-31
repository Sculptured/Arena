//! Runs recorded the way arenad did before integrations: the branch's database `branch_<id>`
//! copied from `world_base`, a `snap_` copy after each committed write, and tool-call events
//! that carry `snapshot` (no `saves`, no `part`). Rows, compare and delete still serve such runs
//! through the old `Worlds` until they move to the parts.
#![allow(dead_code)]

use arenad::{model::*, store::Store};
use serde_json::{json, Value};
use world::snapshot::{Source, Worlds};

struct Legacy<'a> {
    store: &'a Store,
    worlds: &'a Worlds,
    branch: String,
    db: String,
    agent: String,
}

impl Legacy<'_> {
    async fn append(&self, agent: bool, kind: EventKind, payload: Value) {
        let agent = agent.then_some(self.agent.as_str());
        self.store
            .append(&self.branch, agent, kind, payload)
            .await
            .unwrap();
    }

    /// Runs a tool on the branch database, snapshots a committed write, records the call.
    async fn call(&self, i: usize, name: &str, args: &Value) {
        let out = self
            .worlds
            .with_client(&self.db, async |c| world::tools::call(c, name, args).await)
            .await
            .unwrap();
        let snapshot = match out.error.is_none() && !out.writes.is_empty() {
            true => Some(self.worlds.snapshot(&self.db).await.unwrap()),
            false => None,
        };
        let payload = json!({"name": name, "args": args, "result": out.result, "error": out.error,
                             "reads": out.reads, "writes": out.writes, "snapshot": snapshot, "tool_call_id": format!("c{i}")});
        self.append(true, EventKind::ToolCall, payload).await;
    }

    async fn end(&self, status: BranchStatus) {
        let end = json!({"ok": true, "error": null, "tokens": 15, "cost_usd": 0.001, "cost_source": "openrouter", "seconds": 1.0});
        self.append(true, EventKind::AgentEnd, end).await;
        let check = json!({"passed": status == BranchStatus::Passed, "gold_db": "gold_27", "differing_rows": []});
        self.append(false, EventKind::Check, check).await;
        self.store
            .set_status(&self.branch, status, None)
            .await
            .unwrap();
    }
}

impl<'a> Legacy<'a> {
    /// A new run of task 27 with one branch and its database; records `RunStart` and `AgentStart`.
    async fn open(store: &'a Store, worlds: &'a Worlds, team: &Team) -> (String, Legacy<'a>) {
        let run_id = store.create_run("27", team).await.unwrap();
        let branch = store
            .create_branch(&run_id, None, team, &json!([]), None)
            .await
            .unwrap();
        let db = worlds.new_branch(&branch, Source::Base).await.unwrap();
        let agent = team.agents.last().unwrap().id.clone();
        let run = Legacy {
            store,
            worlds,
            branch,
            db,
            agent,
        };
        run.append(
            false,
            EventKind::RunStart,
            json!({"task_id": "27", "team": team}),
        )
        .await;
        let start = json!({"name": run.agent, "kind": "open", "model": null});
        run.append(true, EventKind::AgentStart, start).await;
        (run_id, run)
    }
}

/// Records a finished one-agent run of task 27 whose agent made `calls` (tool name, args).
/// Returns (run id, branch id). Its databases are dropped by the test's `Cleanup` (it sweeps
/// `branch_*` databases of the store's branches and the snapshots their events name).
pub async fn legacy_run(
    store: &Store,
    worlds: &Worlds,
    team: &Team,
    calls: &[(&str, Value)],
    status: BranchStatus,
) -> (String, String) {
    let (run_id, run) = Legacy::open(store, worlds, team).await;
    for (i, (name, args)) in calls.iter().enumerate() {
        run.call(i, name, args).await;
    }
    run.end(status).await;
    (run_id, run.branch)
}
