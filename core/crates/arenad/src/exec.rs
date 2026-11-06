mod links;
mod talk;

use std::{collections::HashMap, sync::Arc, sync::Mutex};

use parts::proto::{Effect, ToolOutput, WorldEvent};
use serde_json::{json, Value};

use crate::{
    clock::BranchClock,
    model::*,
    overhead::{self, Cat},
    recorded::LEGACY_PART,
    store::Store,
    ticker::Ticker,
    world::{Called, PartEvent, Saves, World, WorldError},
};

pub use links::LINK_ACTOR;
pub use talk::{handoff_schema, mail_for, HANDOFF, INBOX, MESSAGE};

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
    /// The rules the branch runs under (from its run's `RunStart`).
    pub rules: Rules,
    /// Agents an external system registered while the branch runs (not in `team`).
    joined: std::sync::RwLock<Vec<AgentSpec>>,
    /// agent id -> its latest handoff note.
    pub handoffs: Mutex<HashMap<String, String>>,
    /// Set when a program of the world has stopped: the branch ends `infra_error` with it.
    fatal: Mutex<Option<String>>,
    /// Held across every tool call so world order, saves and event order agree.
    gate: tokio::sync::Mutex<()>,
    /// Set once something has begun to end the branch.
    ending: std::sync::atomic::AtomicBool,
}

impl BranchCtx {
    /// A branch whose clock starts at 0 (at the speed `ARENA_CLOCK_SPEED` sets), under the rules
    /// of runs recorded before runners (see [`Rules::legacy`]) until `with_rules` says otherwise.
    pub fn new(branch_id: String, world: Arc<World>, team: Team, task_id: String) -> BranchCtx {
        BranchCtx {
            branch_id,
            world,
            team,
            task_id,
            clock: BranchClock::from_env(0),
            ticker: tokio::sync::Mutex::new(None),
            rules: Rules::legacy(),
            joined: Default::default(),
            handoffs: Mutex::new(HashMap::new()),
            fatal: Mutex::new(None),
            gate: tokio::sync::Mutex::new(()),
            ending: Default::default(),
        }
    }

    /// The same branch under `rules`.
    pub fn with_rules(mut self, rules: Rules) -> BranchCtx {
        self.rules = rules;
        self
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

    /// The agent `id`: one of the team, or one that joined while the branch runs.
    pub fn agent(&self, id: &str) -> Option<AgentSpec> {
        let known = self.team.agents.iter().find(|a| a.id == id).cloned();
        known.or_else(|| {
            self.joined
                .read()
                .unwrap()
                .iter()
                .find(|a| a.id == id)
                .cloned()
        })
    }

    /// Every agent: the team's, then those that joined, in order.
    pub fn agents(&self) -> Vec<AgentSpec> {
        let mut all = self.team.agents.clone();
        all.extend(self.joined.read().unwrap().iter().cloned());
        all
    }

    /// Adds an agent that joined while the branch runs; false when one has its id already.
    pub fn join(&self, spec: AgentSpec) -> bool {
        let mut joined = self.joined.write().unwrap();
        let taken = self
            .team
            .agents
            .iter()
            .chain(joined.iter())
            .any(|a| a.id == spec.id);
        if !taken {
            joined.push(spec);
        }
        !taken
    }

    /// True when `agent`'s team keeps it from writing (an unknown agent writes nothing either).
    pub fn read_only(&self, agent: &str) -> bool {
        self.agent(agent).is_none_or(|a| a.read_only)
    }

    pub fn has_agent(&self, agent: &str) -> bool {
        self.agent(agent).is_some()
    }

    /// The provider `agent`'s model calls go to (`Some(None)`: it names none, so the default
    /// one); `None` when the branch has no such agent.
    pub fn provider_of(&self, agent: &str) -> Option<Option<String>> {
        self.agent(agent).map(|a| a.provider)
    }

    pub fn handoff_note(&self, agent: &str) -> Option<String> {
        self.handoffs.lock().unwrap().get(agent).cloned()
    }

    /// Holds the branch gate: no tool call runs on the world until the guard drops.
    pub async fn gate(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.gate.lock().await
    }

    /// Records an event of this branch with the world clock paused: recording is arena's work.
    pub async fn append(
        &self,
        store: &Store,
        agent: Option<&str>,
        kind: EventKind,
        payload: Value,
    ) -> anyhow::Result<Event> {
        let _paused = self.clock.paused();
        store.append(&self.branch_id, agent, kind, payload).await
    }

    /// Waits for the branch gate, counting the wait as arena's overhead.
    async fn gate_timed(&self) -> tokio::sync::MutexGuard<'_, ()> {
        let since = std::time::Instant::now();
        let guard = self.gate.lock().await;
        overhead::add_since(&self.branch_id, Cat::Wait, since);
        guard
    }

    /// True for the one caller that may end the branch: the first to ask.
    pub fn claim_end(&self) -> bool {
        !self.ending.swap(true, std::sync::atomic::Ordering::SeqCst)
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

/// The tools `agent` gets on `branch`, in MCP shape (`name`, `description`, `inputSchema`): the
/// world's (only its read tools when the agent's team made it read-only), and the tools agents
/// talk with (see `talk`).
pub fn tool_schemas(branch: &BranchCtx, agent: &str) -> Vec<Value> {
    let read_only = branch.read_only(agent);
    let mut list: Vec<Value> = branch
        .world
        .tools()
        .filter(|t| !read_only || t.effect != Effect::Write)
        .map(|t| json!({"name": t.name, "description": t.description, "inputSchema": t.input_schema}))
        .collect();
    list.extend(talk::schemas(branch, agent));
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
    duration_ms: Option<f64>,
    /// Events the call gave the world, reported after the call is recorded.
    events: Vec<PartEvent>,
}

impl CallFacts {
    /// Takes what a world call gave back besides its outcome: its part, reads, writes and events.
    fn take(&mut self, called: Called, clock_ms: i64) {
        let Called { output, part, .. } = called;
        self.events = part_events(part.as_deref(), output.events, clock_ms);
        (self.part, self.reads, self.writes) = (part, output.reads, output.writes);
    }
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
        duration_ms: f.duration_ms,
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

/// `events` a tool call on `part` gave at `clock_ms`, as part events.
fn part_events(part: Option<&str>, events: Box<[WorldEvent]>, clock_ms: i64) -> Vec<PartEvent> {
    let Some(part) = part else {
        return vec![];
    };
    events
        .into_vec()
        .into_iter()
        .map(|event| PartEvent {
            part: part.to_string(),
            event,
            clock_ms,
        })
        .collect()
}

/// Why `agent` may not run `name` now: the branch can't go on, or it writes and the agent's team
/// made it read-only.
fn refusal(branch: &BranchCtx, agent: &str, name: &str) -> Option<String> {
    if let Some(f) = branch.fatal() {
        return Some(f);
    }
    (branch.world.is_write(name) && branch.read_only(agent)).then(|| {
        "Write tools are not available to this agent: its team made it read-only".to_string()
    })
}

/// Saves what changed with a step (the part written to, the clock parts that changed) with the
/// clock paused, and records the saves, a failure and the pause.
async fn save_step(branch: &BranchCtx, written: Option<&str>, facts: &mut CallFacts) {
    let paused = branch.clock.paused();
    let step = branch.world.save_changed(written).await;
    if !step.saves.is_empty() || step.error.is_some() {
        facts.paused_ms = Some(paused.elapsed_ms());
        overhead::add(&branch.branch_id, Cat::Save, paused.elapsed_ms_f64());
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

    /// Advances the clock parts to `clock_ms` and reports the events they give.
    async fn advance(&self, branch: &BranchCtx, clock_ms: i64) -> Result<(), String> {
        let events = branch
            .world
            .advance_all(clock_ms)
            .await
            .map_err(|e| failure(branch, e))?;
        self.report(branch, events).await
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
        let since = std::time::Instant::now();
        let called = match branch.world.call_tool(name, args.clone(), clock).await {
            Ok(c) => c,
            Err(e) => return (Err(failure(branch, e)), facts),
        };
        let gate_ms = called.gate_ms;
        facts.duration_ms = Some((since.elapsed().as_secs_f64() * 1000.0 - gate_ms).max(0.0));
        overhead::add(&branch.branch_id, Cat::Gate, gate_ms);
        let outcome = outcome_of(&called.output);
        facts.take(called, clock);
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
        let _gate = branch.gate_timed().await;
        if talk::is_talk(branch, name) {
            return self.talk(branch, agent, name, args, tool_call_id).await;
        }
        let (outcome, mut facts) = self.execute(branch, agent, name, &args).await;
        let events = std::mem::take(&mut facts.events);
        let payload = tool_call_payload(name, args, &outcome, tool_call_id, facts);
        self.record(branch, Some(agent), EventKind::ToolCall, &payload)
            .await?;
        self.report(branch, events).await?;
        outcome
    }

    async fn record<T: serde::Serialize>(
        &self,
        branch: &BranchCtx,
        agent: Option<&str>,
        kind: EventKind,
        payload: &T,
    ) -> Result<(), String> {
        let v = serde_json::to_value(payload).map_err(|e| format!("internal error: {e}"))?;
        branch
            .append(&self.store, agent, kind, v)
            .await
            .map_err(|e| format!("internal error: recording event: {e:#}"))?;
        Ok(())
    }
}
