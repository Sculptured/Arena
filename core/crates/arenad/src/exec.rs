use std::{collections::HashMap, sync::Arc, sync::Mutex};

use serde_json::{json, Value};
use world::{snapshot::Worlds, tools};

use crate::{model::*, store::Store};

/// One running branch: its database, team and per-agent handoff notes.
pub struct BranchCtx {
    pub branch_id: String,
    pub db: String,
    pub team: Team,
    pub task_id: String,
    /// agent id -> handoff note, set once the agent has called `handoff`.
    pub handoffs: Mutex<HashMap<String, String>>,
}

impl BranchCtx {
    pub fn new(branch_id: String, db: String, team: Team, task_id: String) -> BranchCtx {
        BranchCtx { branch_id, db, team, task_id, handoffs: Mutex::new(HashMap::new()) }
    }

    pub fn is_last(&self, agent: &str) -> bool {
        self.team.agents.last().is_some_and(|a| a.id == agent)
    }

    pub fn has_agent(&self, agent: &str) -> bool {
        self.team.agents.iter().any(|a| a.id == agent)
    }

    pub fn handoff_note(&self, agent: &str) -> Option<String> {
        self.handoffs.lock().unwrap().get(agent).cloned()
    }
}

/// The single place where tool calls are executed, recorded and snapshotted.
pub struct ToolExec {
    pub store: Arc<Store>,
    pub worlds: Arc<Worlds>,
}

pub fn handoff_schema() -> Value {
    json!({"name": "handoff",
           "description": "Pass the conversation to the next agent with a note summarising what you learned and what remains. Call it exactly once, when you are done.",
           "inputSchema": {"type": "object",
                           "properties": {"note": {"type": "string"}},
                           "required": ["note"]}})
}

impl ToolExec {
    pub fn new(store: Arc<Store>, worlds: Arc<Worlds>) -> ToolExec {
        ToolExec { store, worlds }
    }

    /// The JSON given back to the agent: the result, or `{"error": msg}`.
    pub async fn run(
        &self,
        branch: &BranchCtx,
        agent: &str,
        name: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Value {
        match self.run_outcome(branch, agent, name, args, tool_call_id).await {
            Ok(v) => v,
            Err(e) => json!({"error": e}),
        }
    }

    /// Like `run`, but keeps success and tool error apart.
    pub async fn run_outcome(
        &self,
        branch: &BranchCtx,
        agent: &str,
        name: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        if name == "handoff" {
            return self.handoff(branch, agent, args, tool_call_id).await;
        }
        let last = branch.is_last(agent);
        let mut snapshot = None;
        let mut reads = vec![];
        let mut writes = vec![];
        let outcome: Result<Value, String> = if tools::WRITE_TOOLS.contains(&name) && !last {
            Err(format!("{name} is not available to you: only the last agent in the team may make changes"))
        } else {
            match self
                .worlds
                .with_client(&branch.db, async |c| tools::call(c, name, &args).await)
                .await
            {
                Err(e) => Err(format!("internal error: {e:#}")),
                Ok(o) => {
                    reads = o.reads;
                    writes = o.writes;
                    match o.error {
                        Some(e) => Err(e),
                        None => {
                            if !writes.is_empty() {
                                match self.worlds.snapshot(&branch.db).await {
                                    Ok(s) => snapshot = Some(s),
                                    Err(e) => eprintln!("snapshot of {} failed: {e:#}", branch.db),
                                }
                            }
                            Ok(o.result.unwrap_or(Value::Null))
                        }
                    }
                }
            }
        };
        let payload = ToolCallPayload {
            name: name.to_string(),
            args,
            result: outcome.as_ref().ok().cloned().unwrap_or(Value::Null),
            error: outcome.as_ref().err().cloned(),
            reads: json!(reads),
            writes: json!(writes),
            snapshot,
            tool_call_id,
        };
        self.record(branch, agent, EventKind::ToolCall, &payload).await?;
        outcome
    }

    async fn handoff(
        &self,
        branch: &BranchCtx,
        agent: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        let note = args.get("note").and_then(Value::as_str).map(str::trim).unwrap_or("");
        let outcome: Result<String, String> = if branch.is_last(agent) {
            Err("handoff is not available to the last agent".to_string())
        } else if note.is_empty() {
            Err("handoff() requires a non-empty 'note' string".to_string())
        } else {
            let mut h = branch.handoffs.lock().unwrap();
            if h.contains_key(agent) {
                Err("handoff already sent".to_string())
            } else {
                h.insert(agent.to_string(), note.to_string());
                Ok(note.to_string())
            }
        };
        match outcome {
            Ok(note) => {
                self.record(branch, agent, EventKind::Handoff, &HandoffPayload { note }).await?;
                Ok(json!({"ok": true}))
            }
            Err(e) => {
                let payload = ToolCallPayload {
                    name: "handoff".into(),
                    args,
                    result: Value::Null,
                    error: Some(e.clone()),
                    reads: json!([]),
                    writes: json!([]),
                    snapshot: None,
                    tool_call_id,
                };
                self.record(branch, agent, EventKind::ToolCall, &payload).await?;
                Err(e)
            }
        }
    }

    async fn record<T: serde::Serialize>(
        &self,
        branch: &BranchCtx,
        agent: &str,
        kind: EventKind,
        payload: &T,
    ) -> Result<(), String> {
        let v = serde_json::to_value(payload).map_err(|e| format!("internal error: {e}"))?;
        self.store
            .append(&branch.branch_id, Some(agent), kind, v)
            .await
            .map_err(|e| format!("internal error: recording event: {e:#}"))?;
        Ok(())
    }
}
