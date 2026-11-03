use std::{collections::HashMap, sync::Arc, sync::Mutex};

use parts::proto::ToolOutput;
use serde_json::{json, Value};

use crate::{
    clock::BranchClock,
    model::*,
    recorded::LEGACY_PART,
    store::Store,
    ticker::Ticker,
    world::{Called, PartEvent, Saves, World, WorldError},
};

/// One running branch: its world, team, arena clock and per-agent handoff notes.
pub struct BranchCtx {
    pub branch_id: String,
    pub world: Arc<World>,
    pub team: Team,
    pub task_id: String,
    /// The branch's arena clock (from 0 at the run's start, from the fork point on a fork).
    pub clock: BranchClock,
    /// Sends `advance` to the world's clock parts while the branch runs.
    ticker: tokio::sync::Mutex<Option<Ticker>>,
    /// agent id -> handoff note, set once the agent has called `handoff`.
    pub handoffs: Mutex<HashMap<String, String>>,
    /// Set when a program of the world has stopped: the branch ends `infra_error` with it.
    fatal: Mutex<Option<String>>,
    /// Held across every tool call so world order, saves and event order agree.
    gate: tokio::sync::Mutex<()>,
}

impl BranchCtx {
    /// A branch whose clock starts at 0 (at the speed `ARENA_CLOCK_SPEED` sets).
    pub fn new(branch_id: String, world: Arc<World>, team: Team, task_id: String) -> BranchCtx {
        BranchCtx {
            branch_id,
            world,
            team,
            task_id,
            clock: BranchClock::from_env(0),
            ticker: tokio::sync::Mutex::new(None),
            handoffs: Mutex::new(HashMap::new()),
            fatal: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
        }
    }

    /// The same branch with `clock` as its arena clock.
    pub fn with_clock(mut self, clock: BranchClock) -> BranchCtx {
        self.clock = clock;
        self
    }

    /// Starts the clock ticker (when the world has clock parts and none runs yet).
    pub async fn start_ticker(self: &Arc<Self>, store: &Arc<Store>) {
        let mut t = self.ticker.lock().await;
        if t.is_none() {
            *t = Ticker::start(ToolExec::new(store.clone()), self.clone());
        }
    }

    /// Stops the clock ticker and waits for it to end.
    pub async fn stop_ticker(&self) {
        if let Some(t) = self.ticker.lock().await.take() {
            t.stop().await;
        }
    }

    pub fn is_last(&self, agent: &str) -> bool {
        self.team.agents.last().is_some_and(|a| a.id == agent)
    }

    pub fn has_agent(&self, agent: &str) -> bool {
        self.team.agents.iter().any(|a| a.id == agent)
    }

    /// The provider `agent`'s model calls go to (`Some(None)`: it names none, so the default
    /// one); `None` when the team has no such agent.
    pub fn provider_of(&self, agent: &str) -> Option<Option<&str>> {
        self.team
            .agents
            .iter()
            .find(|a| a.id == agent)
            .map(|a| a.provider.as_deref())
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
    paused_ms: Option<i64>,
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
        paused_ms: f.paused_ms,
    }
}

/// A tool's outcome as an agent sees it: its result, or its error.
pub fn outcome_of(output: &ToolOutput) -> Result<Value, String> {
    match &output.error {
        Some(e) => Err(e.clone()),
        None => Ok(output.result.clone().unwrap_or(Value::Null)),
    }
}

fn world_event_payload(e: PartEvent) -> WorldEventPayload {
    WorldEventPayload {
        part: e.part,
        kind: e.event.kind,
        text: e.event.text,
        data: e.event.data,
        clock_ms: e.clock_ms,
    }
}

/// Why `agent` may not run `name` now: the branch can't go on, or it writes and the agent is not
/// the last one.
fn refusal(branch: &BranchCtx, agent: &str, name: &str) -> Option<String> {
    if let Some(f) = branch.fatal() {
        return Some(f);
    }
    (branch.world.is_write(name) && !branch.is_last(agent))
        .then(|| "Write tools are not available to this agent".to_string())
}

/// Saves what changed with a step (the part written to, the clock parts that changed) with the
/// clock paused, and records the saves, a failure and the pause.
async fn save_step(branch: &BranchCtx, written: Option<&str>, facts: &mut CallFacts) {
    let paused = branch.clock.paused();
    let step = branch.world.save_changed(written).await;
    if !step.saves.is_empty() || step.error.is_some() {
        facts.paused_ms = Some(paused.elapsed_ms());
    }
    facts.saves = (!step.saves.is_empty()).then_some(step.saves);
    facts.save_error = step.error.map(|e| failure(branch, e));
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

    /// Advances the world's clock parts to the branch clock behind the gate (a clock tick).
    pub async fn tick(&self, branch: &BranchCtx) -> Result<(), String> {
        let _gate = branch.gate().await;
        if branch.fatal().is_some() {
            return Ok(());
        }
        self.advance(branch, branch.clock.now_ms()).await
    }

    /// Advances the clock parts to `clock_ms` and records the events they report.
    async fn advance(&self, branch: &BranchCtx, clock_ms: i64) -> Result<(), String> {
        let events = branch
            .world
            .advance_all(clock_ms)
            .await
            .map_err(|e| failure(branch, e))?;
        for e in events {
            let payload = world_event_payload(e);
            self.record(branch, None, EventKind::WorldEvent, &payload)
                .await?;
        }
        Ok(())
    }

    /// Runs a tool call on the branch's world at the branch clock (the clock parts advanced to
    /// it first), then saves what changed.
    async fn execute(
        &self,
        branch: &BranchCtx,
        agent: &str,
        name: &str,
        args: &Value,
    ) -> (Result<Value, String>, CallFacts) {
        let mut facts = CallFacts::default();
        if let Some(e) = refusal(branch, agent, name) {
            return (Err(e), facts);
        }
        let clock = branch.clock.now_ms();
        facts.clock_ms = Some(clock);
        if let Err(e) = self.advance(branch, clock).await {
            return (Err(e), facts);
        }
        let Called { output, part } = match branch.world.call_tool(name, args.clone(), clock).await
        {
            Ok(c) => c,
            Err(e) => return (Err(failure(branch, e)), facts),
        };
        let outcome = outcome_of(&output);
        (facts.part, facts.reads, facts.writes) = (part, output.reads, output.writes);
        let wrote = outcome.is_ok() && !facts.writes.is_empty();
        let written = facts.part.clone().filter(|_| wrote);
        save_step(branch, written.as_deref(), &mut facts).await;
        (outcome, facts)
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
        let (outcome, facts) = self.execute(branch, agent, name, &args).await;
        let payload = tool_call_payload(name, args, &outcome, tool_call_id, facts);
        self.record(branch, Some(agent), EventKind::ToolCall, &payload)
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
                self.record(
                    branch,
                    Some(agent),
                    EventKind::Handoff,
                    &HandoffPayload { note },
                )
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
                self.record(branch, Some(agent), EventKind::ToolCall, &payload)
                    .await?;
                outcome
            }
        }
    }

    async fn record<T: serde::Serialize>(
        &self,
        branch: &BranchCtx,
        agent: Option<&str>,
        kind: EventKind,
        payload: &T,
    ) -> Result<(), String> {
        let v = serde_json::to_value(payload).map_err(|e| format!("internal error: {e}"))?;
        self.store
            .append(&branch.branch_id, agent, kind, v)
            .await
            .map_err(|e| format!("internal error: recording event: {e:#}"))?;
        Ok(())
    }
}
