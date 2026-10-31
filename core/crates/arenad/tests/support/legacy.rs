//! Runs recorded before integrations, built by hand: a `RunStart` without saves, a tool call that
//! carries `snapshot` (a real `snap_…` database) and no `saves` or `part`, and the final state in
//! the database `branch_<id>`. Legacy names only work without a Postgres namespace, so every
//! database here gets a unique name and `LegacyDbs` drops them, and its save folder, at the end.
#![allow(dead_code)]

use std::path::PathBuf;

use arenad::{model::*, store::Store};
use serde_json::{json, Value};

use crate::pg::{admin, Cleanup};

/// The databases a legacy test makes, dropped on drop, and a save folder for its hub.
pub struct LegacyDbs {
    clean: Cleanup,
    pub saves: PathBuf,
}

impl LegacyDbs {
    pub fn new(tag: &str) -> LegacyDbs {
        let saves = std::env::temp_dir().join(format!("arenad-saves-{tag}"));
        std::fs::create_dir_all(&saves).unwrap();
        LegacyDbs {
            clean: Cleanup::new(),
            saves,
        }
    }

    /// Makes the database `name` as a copy of `template`; it is dropped at the end.
    pub async fn copy(&self, name: &str, template: &str) {
        self.clean.add(name);
        admin().create_db_from(name, template).await.unwrap();
    }

    /// Drops the database `name` at the end (one something else makes).
    pub fn track(&self, name: &str) {
        self.clean.add(name);
    }

    /// Runs `sql` in the database `db`.
    pub async fn execute(&self, db: &str, sql: &str) {
        let url = admin().db_url(db);
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .unwrap();
        let task = tokio::spawn(conn);
        client.batch_execute(sql).await.unwrap();
        drop(client);
        let _ = task.await;
    }
}

impl Drop for LegacyDbs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.saves);
    }
}

fn tool_call(id: &str, name: &str) -> Value {
    json!({"id": id, "type": "function", "function": {"name": name, "arguments": "{}"}})
}

/// A model call the way the open agent recorded it: the messages sent and the reply.
fn model_call(messages: Value, reply: Value) -> Value {
    json!({"request": {"model": "fake/model", "messages": messages},
           "response": {"choices": [{"index": 0, "message": reply, "finish_reason": "stop"}]}})
}

/// The agent's events: a model call chooses the write, the write is snapshotted to `snap`, and
/// a last model call ends the agent.
fn agent_events(agent: &AgentSpec, snap: &str) -> Vec<(EventKind, Value)> {
    let sent =
        json!([{"role": "system", "content": agent.prompt}, {"role": "user", "content": "ticket"}]);
    let chose = json!({"role": "assistant", "content": null,
                       "tool_calls": [tool_call("c1", "exchange_delivered_order_items")]});
    let mut after = sent.as_array().unwrap().clone();
    after.push(chose.clone());
    after.push(json!({"role": "tool", "tool_call_id": "c1", "content": "\"ok\""}));
    let done = json!({"role": "assistant", "content": "Done."});
    let write = json!({"name": "exchange_delivered_order_items", "args": {}, "result": "ok", "error": null,
                       "reads": [], "writes": ["orders/#W7181492"], "snapshot": snap, "tool_call_id": "c1"});
    vec![
        (
            EventKind::AgentStart,
            json!({"name": agent.name, "kind": "open", "model": null}),
        ),
        (EventKind::ModelCall, model_call(sent, chose)),
        (EventKind::ToolCall, write),
        (EventKind::ModelCall, model_call(json!(after), done)),
        (
            EventKind::AgentEnd,
            json!({"ok": true, "error": null, "tokens": 15, "cost_usd": 0.001,
                                     "cost_source": "openrouter", "seconds": 1.0}),
        ),
    ]
}

/// Records a finished one-agent run of task 27 whose write was snapshotted to `snap`; its final
/// database `branch_<id>` is a copy of `final_db`. Returns (run id, branch id).
pub async fn legacy_run(
    store: &Store,
    dbs: &LegacyDbs,
    team: &Team,
    snap: &str,
    final_db: &str,
) -> (String, String) {
    let run_id = store.create_run("27", team).await.unwrap();
    let branch = store
        .create_branch(&run_id, None, team, &json!([]), None)
        .await
        .unwrap();
    for (who, kind, payload) in run_events(team, snap) {
        store.append(&branch, who, kind, payload).await.unwrap();
    }
    let passed = BranchStatus::Passed;
    store.set_status(&branch, passed, None).await.unwrap();
    dbs.copy(&format!("branch_{branch}"), final_db).await;
    (run_id, branch)
}

/// The run's events, `(agent, kind, payload)`: `RunStart`, the last agent's events, `Check`.
fn run_events<'a>(team: &'a Team, snap: &str) -> Vec<(Option<&'a str>, EventKind, Value)> {
    let agent = team.agents.last().unwrap();
    let id = Some(agent.id.as_str());
    let start = json!({"task_id": "27", "team": team});
    let mut events = vec![(None, EventKind::RunStart, start)];
    events.extend(
        agent_events(agent, snap)
            .into_iter()
            .map(|(k, p)| (id, k, p)),
    );
    let check = json!({"passed": true, "gold_db": "gold_27", "differing_rows": []});
    events.push((None, EventKind::Check, check));
    events
}
