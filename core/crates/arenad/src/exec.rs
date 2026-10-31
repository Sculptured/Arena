use std::{collections::HashMap, sync::Arc, sync::Mutex};

use serde_json::{json, Value};

use crate::{
    model::*,
    recorded::LEGACY_PART,
    store::Store,
    world::{Called, Saves, World, WorldError},
};

/// One running branch: its world, team and per-agent handoff notes.
pub struct BranchCtx {
    pub branch_id: String,
    pub world: Arc<World>,
    pub team: Team,
    pub task_id: String,
    /// agent id -> handoff note, set once the agent has called `handoff`.
    pub handoffs: Mutex<HashMap<String, String>>,
    /// Set when a program of the world has stopped: the branch ends `infra_error` with it.
    fatal: Mutex<Option<String>>,
    /// Held across every tool call so world order, saves and event order agree.
    gate: tokio::sync::Mutex<()>,
}

impl BranchCtx {
    pub fn new(branch_id: String, world: Arc<World>, team: Team, task_id: String) -> BranchCtx {
        BranchCtx {
            branch_id,
            world,
            team,
            task_id,
            handoffs: Mutex::new(HashMap::new()),
            fatal: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
        }
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

    /// Holds the branch gate: no tool call runs on the world until the guard drops.
    pub async fn gate(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.lock().await
    }

    /// Why the branch can't go on (a world program stopped), once that has happened.
    pub fn fatal(&self) -> Option<String> {
        self.fatal.lock().unwrap().clone()
    }

    fn set_fatal(&self, msg: &str) {
        self.fatal
            .lock()
            .unwrap()
            .get_or_insert_with(|| msg.to_string());
    }
}

/// The single place where tool calls are executed, recorded and saved.
pub struct ToolExec {
    pub store: Arc<Store>,
}

pub fn handoff_schema() -> Value {
    json!({"name": "handoff",
           "description": "Pass the conversation to the next agent with a note summarising what you learned and what remains. Call it exactly once, when you are done.",
           "inputSchema": {"type": "object",
                           "properties": {"note": {"type": "string"}},
                           "required": ["note"]}})
}

/// The world's tools in MCP shape (`name`, `description`, `inputSchema`), plus `handoff` when asked.
pub fn tool_schemas(world: &World, with_handoff: bool) -> Vec<Value> {
    let mut list: Vec<Value> = world
        .tools()
        .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema}))
        .collect();
    if with_handoff {
        list.push(handoff_schema());
    }
    list
}

/// What running a tool call left to record besides its outcome.
#[derive(Default)]
struct CallFacts {
    part: Option<String>,
    reads: Vec<String>,
    writes: Vec<String>,
    saves: Option<Saves>,
    save_error: Option<String>,
    clock_ms: Option<i64>,
}

fn tool_call_payload(
    name: &str,
    args: Value,
    outcome: &Result<Value, String>,
    tool_call_id: Option<String>,
    f: CallFacts,
) -> ToolCallPayload {
    ToolCallPayload {
        name: name.to_string(),
        args,
        result: outcome.as_ref().ok().cloned().unwrap_or(Value::Null),
        error: outcome.as_ref().err().cloned(),
        reads: json!(f.reads),
        writes: json!(f.writes),
        snapshot: f.saves.as_ref().and_then(|s| s.get(LEGACY_PART).cloned()),
        snapshot_error: f.save_error,
        tool_call_id,
        part: f.part,
        saves: f.saves,
        clock_ms: f.clock_ms,
    }
}

/// Runs a tool call on the branch's world (refusing writes to all but the last agent), and saves
/// what it wrote.
async fn execute(
    branch: &BranchCtx,
    agent: &str,
    name: &str,
    args: &Value,
) -> (Result<Value, String>, CallFacts) {
    let world = &branch.world;
    let mut facts = CallFacts::default();
    if let Some(f) = branch.fatal() {
        return (Err(f), facts);
    }
    if world.is_write(name) && !branch.is_last(agent) {
        return (
            Err("Write tools are not available to this agent".to_string()),
            facts,
        );
    }
    let clock = world.clock_ms();
    facts.clock_ms = Some(clock);
    let Called { output, part } = match world.call_tool(name, args.clone(), clock).await {
        Ok(c) => c,
        Err(e) => return (Err(failure(branch, e)), facts),
    };
    (facts.part, facts.reads, facts.writes) = (part, output.reads, output.writes);
    if let Some(e) = output.error {
        return (Err(e), facts);
    }
    if !facts.writes.is_empty() {
        save_write(branch, &mut facts).await;
    }
    (Ok(output.result.unwrap_or(Value::Null)), facts)
}

/// Saves the part a committed write changed; a failed save is recorded, the call still succeeds.
async fn save_write(branch: &BranchCtx, facts: &mut CallFacts) {
    match branch.world.save_changed(facts.part.as_deref()).await {
        Ok(s) => facts.saves = (!s.is_empty()).then_some(s),
        Err(e) => facts.save_error = Some(failure(branch, e)),
    }
}

/// The message of a failed world call; a stopped program also ends the branch.
fn failure(branch: &BranchCtx, e: WorldError) -> String {
    if e.fatal {
        branch.set_fatal(&e.message);
        e.message
    } else {
        format!("internal error: {e}")
    }
}

/// Records `agent`'s handoff note, or says why it can't.
fn take_handoff(branch: &BranchCtx, agent: &str, args: &Value) -> Result<String, String> {
    let note = args
        .get("note")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if branch.is_last(agent) {
        return Err("handoff is not available to the last agent".to_string());
    }
    if note.is_empty() {
        return Err("handoff() requires a non-empty 'note' string".to_string());
    }
    let mut h = branch.handoffs.lock().unwrap();
    if h.contains_key(agent) {
        return Err("handoff already sent".to_string());
    }
    h.insert(agent.to_string(), note.to_string());
    Ok(note.to_string())
}

impl ToolExec {
    pub fn new(store: Arc<Store>) -> ToolExec {
        ToolExec { store }
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
        match self
            .run_outcome(branch, agent, name, args, tool_call_id)
            .await
        {
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
        let _gate = branch.gate().await;
        if name == "handoff" {
            return self.handoff(branch, agent, args, tool_call_id).await;
        }
        let (outcome, facts) = execute(branch, agent, name, &args).await;
        let payload = tool_call_payload(name, args, &outcome, tool_call_id, facts);
        self.record(branch, agent, EventKind::ToolCall, &payload)
            .await?;
        outcome
    }

    async fn handoff(
        &self,
        branch: &BranchCtx,
        agent: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        match take_handoff(branch, agent, &args) {
            Ok(note) => {
                self.record(branch, agent, EventKind::Handoff, &HandoffPayload { note })
                    .await?;
                Ok(json!({"ok": true}))
            }
            Err(e) => {
                let outcome = Err(e);
                let payload = tool_call_payload(
                    "handoff",
                    args,
                    &outcome,
                    tool_call_id,
                    CallFacts::default(),
                );
                self.record(branch, agent, EventKind::ToolCall, &payload)
                    .await?;
                outcome
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
