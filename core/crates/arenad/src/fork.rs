//! Fork: go back to step N of a branch, change components, and rerun from there with the
//! world restored as it was just before step N: exactly (from saves, or by replay), or
//! approximately when asked.

mod exact;
mod rerun;

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

use serde::Serialize;
use serde_json::Value;

use crate::{
    app::AppState,
    clock::BranchClock,
    exec::BranchCtx,
    model::*,
    providers::ProviderConfig,
    recorded::{run_info, saves_at_except},
    restore::{restored, Refused},
    runner::{self, StartPoint},
    world::Saves,
};

pub use rerun::{part_plans, prepare, ForkRequest, PartPlan, Rerun};

pub const REASON_EARLIER: &str = "a change targets an agent that finished earlier";
pub const REASON_CLI: &str = "Claude Code and Codex sessions restart from the agent's start";
pub const REASON_KIND: &str = "the agent's mode changes";
pub const REASON_TOOL: &str = "a tool call is redone from the model call that chose it";
pub const REASON_RUN_START: &str = "the run restarts from the first agent's start";
pub const REASON_AGENT_RESTART: &str = "the agent restarts from its first start";
pub const REASON_LAST_MODEL_CALL: &str =
    "the step is redone from the agent's last model call before it";
pub const REASON_NO_MODEL_CALL: &str =
    "the agent made no model call to resume from, so it restarts from its start";

/// Where a fork actually starts, and what it carries over.
#[derive(Debug, Clone, PartialEq)]
pub struct ForkPlan {
    pub start_seq: i64,
    /// `Some(at_seq)` when `start_seq != at_seq`.
    pub moved_from: Option<i64>,
    /// Why the fork point moved (None when it didn't).
    pub reason: Option<String>,
    pub agent_index: usize,
    /// Per part, the save the new branch's world starts from.
    pub restore: Saves,
    pub resume: Option<Vec<Value>>,
    /// Recorded handoff notes of the agents before `agent_index`, in chain order.
    pub notes: Vec<String>,
}

#[derive(Debug)]
pub enum ForkError {
    /// No event with that seq on the branch (or the timeline lacks a step the plan needs).
    UnknownSeq,
    /// No branch with that id.
    UnknownBranch(String),
    /// A change names an agent that isn't in the team (or an event names one).
    UnknownAgent(String),
    /// The source branch is still running.
    BranchRunning,
    /// A successful write at `seq` has no save, so the world before the fork point
    /// can't be restored exactly.
    NoRestorePoint { seq: i64 },
    /// An agent before the fork point has no recorded handoff note.
    MissingHandoff(String),
    /// A part can't be restored in the mode asked for.
    Refused(Refused),
    /// Replaying a part gave back something else at step `seq` than the first time.
    ReplayDiverged { part: String, seq: i64 },
    /// A part to replay runs on a different machine or version than when the run started.
    FingerprintMismatch { part: String },
    /// Storage or database failure.
    Internal(anyhow::Error),
}

impl std::fmt::Display for ForkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ForkError::UnknownSeq => write!(f, "there is no step with that number on this branch"),
            ForkError::UnknownBranch(b) => write!(f, "there is no branch \"{b}\""),
            ForkError::UnknownAgent(a) => write!(f, "there is no agent \"{a}\" in this team"),
            ForkError::BranchRunning => write!(f, "the branch is still running"),
            ForkError::NoRestorePoint { seq } => write!(
                f,
                "the write at step {seq} has no snapshot, so the world before this step can't be restored exactly"
            ),
            ForkError::MissingHandoff(a) => write!(f, "agent \"{a}\" has no recorded handoff note to carry over"),
            ForkError::Refused(r) => write!(f, "{r}"),
            ForkError::ReplayDiverged { part, seq } => write!(
                f,
                "replay of {part} diverged at step {seq}, so it can't be restored exactly"
            ),
            ForkError::FingerprintMismatch { part } => write!(
                f,
                "the {part} integration is running on a different machine or version than the original"
            ),
            ForkError::Internal(e) => write!(f, "fork failed: {e:#}"),
        }
    }
}

impl std::error::Error for ForkError {}

impl From<anyhow::Error> for ForkError {
    fn from(e: anyhow::Error) -> ForkError {
        ForkError::Internal(e)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ForkResponse {
    pub branch_id: String,
    pub start_seq: i64,
    pub moved_from: Option<i64>,
    pub reason: Option<String>,
}

/// True when `c` changes the agent's mode (a kind equal to the current one is no change).
fn changes_kind(c: &Change, spec: &AgentSpec) -> bool {
    c.kind.is_some_and(|k| k != spec.kind)
}

/// A text a change sets: `Some(None)` clears it (empty or blank text), `Some(Some(t))` sets it.
fn new_text(text: Option<&String>) -> Option<Option<String>> {
    text.map(|t| trimmed(t))
}

/// The model a change sets (see `new_text`).
fn new_model(c: &Change) -> Option<Option<String>> {
    new_text(c.model.as_ref())
}

/// True when `c` changes the agent's model (the same model, or clearing an unset one, is no change).
fn changes_model(c: &Change, spec: &AgentSpec) -> bool {
    new_model(c).is_some_and(|m| m != spec.model)
}

/// The provider a change sets (see `new_text`; cleared is back to the default one).
fn new_provider(c: &Change) -> Option<Option<String>> {
    new_text(c.provider.as_ref())
}

/// True when `c` names another provider than the agent's. Changes are settled first
/// ([`settle_changes`]), so a name for the agent's own provider is no longer there.
fn changes_provider(c: &Change, spec: &AgentSpec) -> bool {
    new_provider(c).is_some_and(|p| p != spec.provider)
}

/// Drops each provider change that names the agent's own provider (as `providers` resolves
/// both: absent is the default one), so the fork rules see only real ones.
pub fn settle_changes(
    team: &Team,
    changes: &mut HashMap<String, Change>,
    providers: &ProviderConfig,
) {
    for spec in &team.agents {
        let Some(c) = changes.get_mut(&spec.id) else {
            continue;
        };
        let same = new_provider(c).is_some_and(|p| {
            providers.id_of(p.as_deref()) == providers.id_of(spec.provider.as_deref())
        });
        if same {
            c.provider = None;
        }
    }
}

/// A provider change is a model change: the fork rules treat both alike.
fn is_change(c: &Change, spec: &AgentSpec) -> bool {
    c.prompt.is_some()
        || changes_model(c, spec)
        || changes_provider(c, spec)
        || changes_kind(c, spec)
}

/// Where the new branch starts, and the resume messages it carries (open agents only).
struct Start {
    seq: i64,
    reason: Option<&'static str>,
    agent: usize,
    resume: Option<Vec<Value>>,
}

/// What a plan reads: the branch's events and team, and the fork point.
struct Planner<'a> {
    events: &'a [Event],
    team: &'a Team,
    at_seq: i64,
    at: &'a Event,
}

/// The index of the agent `id` in `team`.
fn index_of(team: &Team, id: &str) -> Option<usize> {
    team.agents.iter().position(|a| a.id == id)
}

impl Planner<'_> {
    /// An agent's real start: its first AgentStart (a resumed agent has a second one).
    fn first_start(&self, i: usize) -> Result<i64, ForkError> {
        let id = self.team.agents[i].id.as_str();
        self.events
            .iter()
            .find(|e| e.kind == EventKind::AgentStart && e.agent.as_deref() == Some(id))
            .map(|e| e.seq)
            .ok_or(ForkError::UnknownSeq)
    }

    /// An agent has finished at `at_seq` once its AgentEnd is at or before it.
    fn finished(&self, i: usize) -> bool {
        let id = self.team.agents[i].id.as_str();
        self.events.iter().any(|e| {
            e.seq <= self.at_seq && e.kind == EventKind::AgentEnd && e.agent.as_deref() == Some(id)
        })
    }

    /// 1. The agent active at `at_seq`.
    fn active_agent(&self) -> Result<usize, ForkError> {
        let last_start = self
            .events
            .iter()
            .rev()
            .find(|e| e.seq <= self.at_seq && e.kind == EventKind::AgentStart);
        match (self.at.kind, last_start) {
            (EventKind::RunStart, _) | (_, None) => Ok(0),
            (_, Some(s)) => {
                let id = s.agent.clone().unwrap_or_default();
                index_of(self.team, &id).ok_or(ForkError::UnknownAgent(id))
            }
        }
    }

    /// Restart agent `i` from its first start, for `reason`.
    fn restart(&self, i: usize, reason: &'static str) -> Result<Start, ForkError> {
        Ok(Start {
            seq: self.first_start(i)?,
            reason: Some(reason),
            agent: i,
            resume: None,
        })
    }

    /// 2 to 4. Where the fork starts, given the agents `changed` and the active agent `a`.
    fn start_for(
        &self,
        changes: &HashMap<String, Change>,
        changed: &[usize],
        a: usize,
    ) -> Result<Start, ForkError> {
        // A change to an agent that has finished (including `a`): restart from it.
        let earliest = changed
            .iter()
            .copied()
            .filter(|&i| i < a || self.finished(i))
            .min();
        if let Some(e) = earliest {
            return self.restart(e, REASON_EARLIER);
        }
        let spec = &self.team.agents[a];
        // Mode change, or a CLI agent: restart the agent.
        if changes.get(&spec.id).is_some_and(|c| changes_kind(c, spec)) {
            return self.restart(a, REASON_KIND);
        }
        if spec.kind != AgentKind::Open {
            return self.restart(a, REASON_CLI);
        }
        match self.at.kind {
            // Begin with the first agent (or right after RunStart if it never began).
            EventKind::RunStart => Ok(Start {
                seq: self.first_start(0).unwrap_or(self.at_seq + 1),
                reason: Some(REASON_RUN_START),
                agent: a,
                resume: None,
            }),
            EventKind::AgentStart => self.restart(a, REASON_AGENT_RESTART),
            _ => self.resume_from_model_call(a),
        }
    }

    /// The model call an open agent resumes from: the one at the fork point, else the agent's
    /// last one before it.
    fn model_call_before(&self, a: usize, agent_start: i64) -> Option<&Event> {
        if self.at.kind == EventKind::ModelCall {
            return Some(self.at);
        }
        let id = self.team.agents[a].id.as_str();
        self.events.iter().rev().find(|e| {
            e.seq <= self.at_seq
                && e.seq > agent_start
                && e.kind == EventKind::ModelCall
                && e.agent.as_deref() == Some(id)
        })
    }

    /// 4. Open agent: resume from a model call.
    fn resume_from_model_call(&self, a: usize) -> Result<Start, ForkError> {
        let agent_start = self.first_start(a)?;
        let model_call = self.model_call_before(a, agent_start);
        let messages =
            model_call.and_then(|m| m.payload["request"]["messages"].as_array().cloned());
        let (seq, reason, resume) = match (model_call, messages) {
            (Some(m), Some(msgs)) => {
                let why = match self.at.kind {
                    EventKind::ModelCall => None,
                    EventKind::ToolCall | EventKind::Handoff => Some(REASON_TOOL),
                    _ => Some(REASON_LAST_MODEL_CALL),
                };
                (m.seq, why, Some(msgs))
            }
            _ => (agent_start, Some(REASON_NO_MODEL_CALL), None),
        };
        Ok(Start {
            seq,
            reason,
            agent: a,
            resume,
        })
    }

    /// 6. The recorded handoff notes of the agents before `a`.
    fn handoff_notes(&self, a: usize, start_seq: i64) -> Result<Vec<String>, ForkError> {
        self.team.agents[..a]
            .iter()
            .map(|spec| {
                self.events
                    .iter()
                    .rev()
                    .find(|e| {
                        e.seq < start_seq
                            && e.kind == EventKind::Handoff
                            && e.agent.as_deref() == Some(spec.id.as_str())
                    })
                    .and_then(|e| e.payload["note"].as_str().map(str::to_string))
                    .ok_or_else(|| ForkError::MissingHandoff(spec.id.clone()))
            })
            .collect()
    }
}

/// The agents (by index) that `changes` really changes; an unknown agent is an error.
fn changed_agents(team: &Team, changes: &HashMap<String, Change>) -> Result<Vec<usize>, ForkError> {
    let mut changed = vec![];
    for (id, c) in changes {
        let i = index_of(team, id).ok_or_else(|| ForkError::UnknownAgent(id.clone()))?;
        if is_change(c, &team.agents[i]) {
            changed.push(i);
        }
    }
    Ok(changed)
}

/// Plans a fork of a branch (its full event list, `team` its effective team) at `at_seq`.
/// Empty changes (no field set, or only the agent's current kind) are ignored.
pub fn plan_fork(
    events: &[Event],
    team: &Team,
    at_seq: i64,
    changes: &HashMap<String, Change>,
) -> Result<ForkPlan, ForkError> {
    plan_fork_with(events, team, at_seq, changes, &BTreeSet::new())
}

/// `plan_fork` for a world whose `replayed` parts are restored by replay: they need no save.
pub fn plan_fork_with(
    events: &[Event],
    team: &Team,
    at_seq: i64,
    changes: &HashMap<String, Change>,
    replayed: &BTreeSet<String>,
) -> Result<ForkPlan, ForkError> {
    if team.agents.is_empty() {
        return Err(ForkError::UnknownAgent(String::new()));
    }
    let changed = changed_agents(team, changes)?;
    let at = events
        .iter()
        .find(|e| e.seq == at_seq)
        .ok_or(ForkError::UnknownSeq)?;
    let planner = Planner {
        events,
        team,
        at_seq,
        at,
    };
    let active = planner.active_agent()?;
    let mut start = planner.start_for(changes, &changed, active)?;
    let a = start.agent;
    if start.seq == planner.first_start(a).unwrap_or(i64::MIN) {
        start.resume = None;
    }
    // 5. The world just before the start: per part, the save of its last write before it.
    let restore = saves_at_except(events, start.seq - 1, &run_info(events).base, replayed)
        .map_err(|n| ForkError::NoRestorePoint { seq: n.seq })?;
    let notes = planner.handoff_notes(a, start.seq)?;
    Ok(finish_plan(start, at_seq, restore, notes))
}

fn finish_plan(start: Start, at_seq: i64, restore: Saves, notes: Vec<String>) -> ForkPlan {
    let moved = start.seq != at_seq;
    ForkPlan {
        start_seq: start.seq,
        moved_from: moved.then_some(at_seq),
        reason: start.reason.filter(|_| moved).map(str::to_string),
        agent_index: start.agent,
        restore,
        resume: start.resume,
        notes,
    }
}

/// `team` with each agent's prompt/model/kind/provider replaced where `changes` sets them.
pub fn apply_changes(team: &Team, changes: &HashMap<String, Change>) -> Team {
    let mut t = team.clone();
    for spec in &mut t.agents {
        if let Some(c) = changes.get(&spec.id) {
            apply_change(spec, c);
        }
    }
    t
}

fn apply_change(spec: &mut AgentSpec, c: &Change) {
    if let Some(p) = &c.prompt {
        spec.prompt = p.clone();
    }
    if changes_kind(c, spec) || changes_provider(c, spec) {
        // The old model belongs to the old driver or provider: the new one's default applies.
        spec.model = None;
    }
    if let Some(m) = new_model(c) {
        spec.model = m;
    }
    if let Some(p) = new_provider(c) {
        spec.provider = p;
    }
    if let Some(k) = c.kind {
        spec.kind = k;
    }
}

/// Forks `branch_id` at `at_seq` with `changes`, restoring exactly, and starts the new branch.
pub async fn fork(
    state: &Arc<AppState>,
    branch_id: &str,
    at_seq: i64,
    changes: HashMap<String, Change>,
) -> Result<ForkResponse, ForkError> {
    let restore = parts::proto::Mode::Exact;
    let req = ForkRequest {
        at_seq,
        changes,
        restore,
    };
    fork_with(state, branch_id, req).await
}

/// Forks `branch_id` as `req` asks and starts the new branch running. A refusal comes before
/// anything is recorded; a replay that fails ends the new branch `infra_error`.
pub async fn fork_with(
    state: &Arc<AppState>,
    branch_id: &str,
    mut req: ForkRequest,
) -> Result<ForkResponse, ForkError> {
    let parent = finished_branch(state, branch_id).await?;
    settle_changes(&parent.team, &mut req.changes, &state.config.providers);
    let events = state.store.events(branch_id).await?;
    let mut rerun = prepare(state, &parent, &events, &req).await?;
    let task_id = task_of(state, &events, &parent).await?;
    let team = apply_changes(&parent.team, &req.changes);
    let new_id = create_child(state, &parent, &rerun, &team, &req).await?;
    let world = start_child_world(state, &events, &parent.id, &new_id, &mut rerun).await?;
    let ctx = BranchCtx::new(new_id, Arc::new(world), team, task_id)
        .with_clock(BranchClock::from_env(rerun.clock_ms));
    Ok(spawn_child(state, ctx, rerun.plan))
}

/// How each part of a fork of `branch_id` at `at_seq` would be restored in `mode` (what the
/// rerun panel shows before forking).
pub async fn restore_preview(
    state: &AppState,
    branch_id: &str,
    at_seq: i64,
    restore: parts::proto::Mode,
) -> Result<Vec<PartPlan>, ForkError> {
    let branch = state
        .store
        .find_branch(branch_id)
        .await?
        .ok_or_else(|| ForkError::UnknownBranch(branch_id.to_string()))?;
    let events = state.store.events(branch_id).await?;
    let req = ForkRequest {
        at_seq,
        changes: HashMap::new(),
        restore,
    };
    let rerun = prepare(state, &branch, &events, &req).await?;
    Ok(part_plans(&rerun, &events))
}

/// Runs the new branch from the plan's start point; the response names where it started.
fn spawn_child(state: &Arc<AppState>, ctx: BranchCtx, plan: ForkPlan) -> ForkResponse {
    let ForkPlan {
        start_seq,
        moved_from,
        reason,
        agent_index,
        resume,
        notes,
        ..
    } = plan;
    let branch_id = ctx.branch_id.clone();
    runner::spawn_branch(
        state,
        Arc::new(ctx),
        StartPoint {
            agent_index,
            resume,
            notes,
        },
    );
    ForkResponse {
        branch_id,
        start_seq,
        moved_from,
        reason,
    }
}

/// The branch to fork: it must exist and have finished.
async fn finished_branch(state: &AppState, branch_id: &str) -> Result<Branch, ForkError> {
    match state.store.find_branch(branch_id).await? {
        None => Err(ForkError::UnknownBranch(branch_id.to_string())),
        Some(b) if b.status == BranchStatus::Running => Err(ForkError::BranchRunning),
        Some(b) => Ok(b),
    }
}

/// Records the new branch: the parent's run, the fork point, the effective team, the changes,
/// and how its world is restored.
async fn create_child(
    state: &AppState,
    parent: &Branch,
    rerun: &Rerun,
    team: &Team,
    req: &ForkRequest,
) -> anyhow::Result<String> {
    let changes_json = serde_json::to_value(&req.changes)?;
    let plan = &rerun.plan;
    let from = Some((parent.id.as_str(), plan.start_seq));
    let note = plan.reason.clone();
    let store = &state.store;
    let id = store
        .create_branch(&parent.run_id, from, team, &changes_json, note)
        .await?;
    store
        .set_restore(&id, req.restore, &restored(&rerun.hows()))
        .await?;
    Ok(id)
}

/// The task of the run: from `RunStart`, else from the run row.
async fn task_of(state: &AppState, events: &[Event], parent: &Branch) -> anyhow::Result<String> {
    if let Some(t) = events
        .iter()
        .find(|e| e.kind == EventKind::RunStart)
        .and_then(|e| e.payload["task_id"].as_str())
    {
        return Ok(t.to_string());
    }
    let runs = state.store.runs().await?;
    let run = runs.into_iter().find(|r| r.id == parent.run_id);
    run.map(|r| r.task_id)
        .ok_or_else(|| anyhow::anyhow!("no run {} for branch {}", parent.run_id, parent.id))
}

/// Copies the parent's events before the fork point and starts the new branch's world as the
/// rerun plans it, with the run's benchmark and seed. A failure ends the new branch
/// `infra_error` with its message.
async fn start_child_world(
    state: &AppState,
    events: &[Event],
    parent: &str,
    new_id: &str,
    rerun: &mut Rerun,
) -> Result<crate::world::World, ForkError> {
    let started = async {
        let start = rerun.plan.start_seq;
        state.store.copy_prefix(parent, new_id, start).await?;
        rerun::start_world(state, events, new_id, rerun).await
    }
    .await;
    if let Err(e) = &started {
        let error = match e {
            ForkError::Internal(e) => format!("{e:#}"),
            e => e.to_string(),
        };
        runner::setup_failed(state, new_id, error).await;
    }
    started
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(id: &str, kind: AgentKind) -> AgentSpec {
        AgentSpec {
            id: id.into(),
            name: id.into(),
            kind,
            model: None,
            prompt: format!("You are {id}."),
            provider: None,
        }
    }

    fn team(resolver: AgentKind) -> Team {
        Team {
            id: "t".into(),
            name: "T".into(),
            agents: vec![spec("triage", AgentKind::Open), spec("resolver", resolver)],
        }
    }

    fn ev(seq: i64, agent: Option<&str>, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: chrono::Utc::now(),
            agent: agent.map(str::to_string),
            kind,
            payload,
        }
    }

    fn msgs(tag: &str) -> Vec<Value> {
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": tag}),
        ]
    }

    fn mc(seq: i64, agent: &str, tag: &str) -> Event {
        ev(
            seq,
            Some(agent),
            EventKind::ModelCall,
            json!({"request": {"messages": msgs(tag)}, "response": {}, "usage": {}}),
        )
    }

    fn read(seq: i64, agent: &str) -> Event {
        let p = json!({"name": "get_order_details", "args": {}, "result": {}, "error": null,
                       "reads": ["orders/#W1"], "writes": [], "snapshot": null, "tool_call_id": "x"});
        ev(seq, Some(agent), EventKind::ToolCall, p)
    }

    fn write(seq: i64, agent: &str, snapshot: Option<&str>) -> Event {
        let mut p = json!({"name": "exchange_delivered_order_items", "args": {}, "result": {}, "error": null,
                           "reads": ["orders/#W1"], "writes": ["orders/#W1"], "snapshot": snapshot, "tool_call_id": "x"});
        if snapshot.is_none() {
            p["snapshot_error"] = json!("disk full");
        }
        ev(seq, Some(agent), EventKind::ToolCall, p)
    }

    fn start(seq: i64, agent: &str) -> Event {
        ev(seq, Some(agent), EventKind::AgentStart, json!({}))
    }

    /// 1 RunStart; triage 2..=7 (handoff at 6); resolver 8..=14 with writes snapshotted at 10
    /// (snap_a) and 12 (snap_b); 15 Check.
    fn events() -> Vec<Event> {
        vec![
            ev(1, None, EventKind::RunStart, json!({"task_id": "27"})),
            start(2, "triage"),
            mc(3, "triage", "m3"),
            read(4, "triage"),
            mc(5, "triage", "m5"),
            ev(6, Some("triage"), EventKind::Handoff, json!({"note": "N1"})),
            ev(7, Some("triage"), EventKind::AgentEnd, json!({"ok": true})),
            start(8, "resolver"),
            mc(9, "resolver", "m9"),
            write(10, "resolver", Some("snap_a")),
            mc(11, "resolver", "m11"),
            write(12, "resolver", Some("snap_b")),
            mc(13, "resolver", "m13"),
            ev(
                14,
                Some("resolver"),
                EventKind::AgentEnd,
                json!({"ok": true}),
            ),
            ev(15, None, EventKind::Check, json!({"passed": false})),
        ]
    }

    fn base() -> Saves {
        [("retail".to_string(), "world_base".to_string())].into()
    }

    fn snap(s: &str) -> Saves {
        [("retail".to_string(), s.to_string())].into()
    }

    fn no_changes() -> HashMap<String, Change> {
        HashMap::new()
    }

    fn change(agent: &str, c: Change) -> HashMap<String, Change> {
        [(agent.to_string(), c)].into_iter().collect()
    }

    #[test]
    fn cli_agent_moves_to_agent_start() {
        let p = plan_fork(&events(), &team(AgentKind::ClaudeCode), 12, &no_changes()).unwrap();
        assert_eq!(p.start_seq, 8);
        assert_eq!(p.moved_from, Some(12));
        assert_eq!(
            p.reason.as_deref(),
            Some("Claude Code and Codex sessions restart from the agent's start")
        );
        assert_eq!(p.agent_index, 1);
        assert_eq!(p.resume, None);
        assert_eq!(p.restore, base());
        assert_eq!(p.notes, vec!["N1".to_string()]);
    }

    #[test]
    fn open_tool_call_moves_to_its_model_call() {
        let p = plan_fork(&events(), &team(AgentKind::Open), 12, &no_changes()).unwrap();
        assert_eq!(p.start_seq, 11);
        assert_eq!(p.moved_from, Some(12));
        assert_eq!(
            p.reason.as_deref(),
            Some("a tool call is redone from the model call that chose it")
        );
        assert_eq!(p.agent_index, 1);
        assert_eq!(p.resume, Some(msgs("m11")));
        assert_eq!(p.restore, snap("snap_a"));
        assert_eq!(p.notes, vec!["N1".to_string()]);
    }

    #[test]
    fn open_model_call_stays_and_resumes_messages() {
        let p = plan_fork(&events(), &team(AgentKind::Open), 11, &no_changes()).unwrap();
        assert_eq!(p.start_seq, 11);
        assert_eq!(p.moved_from, None);
        assert_eq!(p.reason, None);
        assert_eq!(p.agent_index, 1);
        assert_eq!(p.resume, Some(msgs("m11")));
        assert_eq!(p.restore, snap("snap_a"));
        // A prompt change to the active agent doesn't move the fork point.
        let c = change(
            "resolver",
            Change {
                prompt: Some("New.".into()),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &team(AgentKind::Open), 11, &c).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (11, None));
    }

    #[test]
    fn change_to_finished_agent_moves_back() {
        let c = change(
            "triage",
            Change {
                prompt: Some("Be brief.".into()),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &team(AgentKind::Open), 12, &c).unwrap();
        assert_eq!(p.start_seq, 2);
        assert_eq!(p.moved_from, Some(12));
        assert_eq!(
            p.reason.as_deref(),
            Some("a change targets an agent that finished earlier")
        );
        assert_eq!(p.agent_index, 0);
        assert_eq!(p.resume, None);
        assert_eq!(p.restore, base());
        assert!(p.notes.is_empty());
    }

    fn provider(agent: &str, p: &str) -> HashMap<String, Change> {
        change(
            agent,
            Change {
                provider: Some(p.into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn provider_change_is_a_model_change() {
        let t = team(AgentKind::Open);
        // Like a model change, it resumes the active open agent from the model call ...
        let p = plan_fork(&events(), &t, 11, &provider("resolver", "local:box")).unwrap();
        assert_eq!((p.start_seq, p.moved_from, p.agent_index), (11, None, 1));
        assert_eq!(p.resume, Some(msgs("m11")));
        // ... and moves back to a finished agent it targets.
        let p = plan_fork(&events(), &t, 12, &provider("triage", "openai")).unwrap();
        assert_eq!((p.start_seq, p.agent_index), (2, 0));
        assert_eq!(p.reason.as_deref(), Some(REASON_EARLIER));
        // Clearing an absent provider is no change.
        let p = plan_fork(&events(), &t, 12, &provider("triage", " ")).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (11, Some(12)));
    }

    #[test]
    fn settling_drops_changes_to_the_agents_own_provider() {
        let t = team(AgentKind::Open);
        let both = |a: &str, b: &str| {
            let mut c = provider("triage", a);
            c.extend(provider("resolver", b));
            c
        };
        // Absent is the configured default: openrouter, or local:custom after the override.
        let mut c = both("openrouter", "local:custom");
        settle_changes(&t, &mut c, &ProviderConfig::default());
        assert_eq!(
            (&c["triage"].provider, c["resolver"].provider.as_deref()),
            (&None, Some("local:custom"))
        );
        let mut c = both("openrouter", "local:custom");
        let custom = ProviderConfig::with_upstream("http://127.0.0.1:9");
        settle_changes(&t, &mut c, &custom);
        assert_eq!(
            (c["triage"].provider.as_deref(), &c["resolver"].provider),
            (Some("openrouter"), &None)
        );
        // A blank provider is the default one too.
        let mut c = both(" ", "");
        settle_changes(&t, &mut c, &custom);
        assert_eq!(
            (&c["triage"].provider, &c["resolver"].provider),
            (&None, &None)
        );
    }

    #[test]
    fn apply_changes_provider_rules() {
        let mut t = team(AgentKind::Open);
        t.agents[0].model = Some("old/model".into());
        t.agents[0].provider = Some("local:box".into());
        let ap = |c: Change| apply_changes(&t, &change("triage", c)).agents[0].clone();
        let to = |p: &str| Change {
            provider: Some(p.into()),
            ..Default::default()
        };
        // A new provider drops the old provider's model ...
        let a = ap(to("openai"));
        assert_eq!((a.provider.as_deref(), a.model), (Some("openai"), None));
        // ... unless the change names one.
        let a = ap(Change {
            model: Some("gpt-x".into()),
            ..to("openai")
        });
        assert_eq!(a.model.as_deref(), Some("gpt-x"));
        // The same provider keeps the model; blank is back to the default provider.
        assert_eq!(ap(to("local:box")).model.as_deref(), Some("old/model"));
        let a = ap(to(""));
        assert_eq!((a.provider, a.model), (None, None));
    }

    #[test]
    fn kind_change_moves_to_agent_start() {
        let c = change(
            "resolver",
            Change {
                kind: Some(AgentKind::Codex),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &team(AgentKind::Open), 11, &c).unwrap();
        assert_eq!(p.start_seq, 8);
        assert_eq!(p.moved_from, Some(11));
        assert_eq!(p.reason.as_deref(), Some("the agent's mode changes"));
        assert_eq!(p.agent_index, 1);
        assert_eq!(p.resume, None);
        assert_eq!(p.restore, base());
        assert_eq!(p.notes, vec!["N1".to_string()]);
    }

    #[test]
    fn restore_is_last_snapshot_before_start() {
        let t = team(AgentKind::Open);
        let at = |seq| plan_fork(&events(), &t, seq, &no_changes()).unwrap();
        assert_eq!(at(13).restore, snap("snap_b"));
        assert_eq!(at(12).restore, snap("snap_a")); // moved to 11
        assert_eq!(at(11).restore, snap("snap_a"));
        assert_eq!(at(10).restore, base()); // moved to 9
        assert_eq!(at(9).restore, base());
        assert_eq!(at(3).restore, base());
    }

    #[test]
    fn unsnapshotted_write_has_no_restore_point() {
        let mut evs = events();
        evs[11] = write(12, "resolver", None);
        let e = plan_fork(&evs, &team(AgentKind::Open), 13, &no_changes()).unwrap_err();
        assert!(matches!(e, ForkError::NoRestorePoint { seq: 12 }), "{e:?}");
        // Before the unsnapshotted write, the earlier snapshot is exact.
        let p = plan_fork(&evs, &team(AgentKind::Open), 11, &no_changes()).unwrap();
        assert_eq!(p.restore, snap("snap_a"));
        // A later snapshot is a full copy, so it is exact past an earlier unsnapshotted write.
        let mut evs = events();
        evs[9] = write(10, "resolver", None);
        let p = plan_fork(&evs, &team(AgentKind::Open), 13, &no_changes()).unwrap();
        assert_eq!(p.restore, snap("snap_b"));
    }

    /// The events of a branch forked at seq 11 (resume): prefix 1..=10, then the resumed resolver.
    fn forked_events() -> Vec<Event> {
        let mut evs: Vec<Event> = events().into_iter().take(10).collect();
        evs.extend([
            start(11, "resolver"),
            mc(12, "resolver", "m12"),
            write(13, "resolver", Some("snap_c")),
            mc(14, "resolver", "m14"),
            ev(
                15,
                Some("resolver"),
                EventKind::AgentEnd,
                json!({"ok": true}),
            ),
            ev(16, None, EventKind::Check, json!({"passed": true})),
        ]);
        evs
    }

    #[test]
    fn fork_of_a_fork_with_two_agent_starts() {
        let evs = forked_events();
        let t = team(AgentKind::Open);
        let p = plan_fork(&evs, &t, 13, &no_changes()).unwrap();
        assert_eq!(
            (p.start_seq, p.moved_from, p.agent_index),
            (12, Some(13), 1)
        );
        assert_eq!(p.resume, Some(msgs("m12")));
        assert_eq!(p.restore, snap("snap_a"));
        assert_eq!(p.notes, vec!["N1".to_string()]);
        let p = plan_fork(&evs, &t, 14, &no_changes()).unwrap();
        assert_eq!((p.start_seq, p.restore), (14, snap("snap_c")));
        // Restarting the agent goes to its real (first) start, before any of its writes.
        let c = change(
            "resolver",
            Change {
                kind: Some(AgentKind::ClaudeCode),
                ..Default::default()
            },
        );
        let p = plan_fork(&evs, &t, 13, &c).unwrap();
        assert_eq!((p.start_seq, p.resume, p.restore), (8, None, base()));
        // Forking at the resumed AgentStart restarts the agent from its first start.
        let p = plan_fork(&evs, &t, 11, &no_changes()).unwrap();
        assert_eq!((p.start_seq, p.moved_from, p.resume), (8, Some(11), None));
    }

    #[test]
    fn other_steps_map_to_a_start_point() {
        let t = team(AgentKind::Open);
        // RunStart: the first agent's start.
        let p = plan_fork(&events(), &t, 1, &no_changes()).unwrap();
        assert_eq!(
            (p.start_seq, p.moved_from, p.agent_index, p.resume.clone()),
            (2, Some(1), 0, None)
        );
        assert!(p.reason.is_some());
        // An open agent's AgentStart stays, with no resume.
        let p = plan_fork(&events(), &t, 8, &no_changes()).unwrap();
        assert_eq!(
            (
                p.start_seq,
                p.moved_from,
                p.reason.clone(),
                p.resume.clone()
            ),
            (8, None, None, None)
        );
        assert_eq!(p.notes, vec!["N1".to_string()]);
        // A handoff is a tool call: redone from the model call that chose it.
        let p = plan_fork(&events(), &t, 6, &no_changes()).unwrap();
        assert_eq!((p.start_seq, p.agent_index), (5, 0));
        assert_eq!(
            p.reason.as_deref(),
            Some("a tool call is redone from the model call that chose it")
        );
        assert_end_steps_map_to_last_model_call(&t);
    }

    /// AgentEnd and Check: from the agent's last model call.
    fn assert_end_steps_map_to_last_model_call(t: &Team) {
        let p = plan_fork(&events(), t, 14, &no_changes()).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (13, Some(14)));
        let p = plan_fork(&events(), t, 15, &no_changes()).unwrap();
        assert_eq!(
            (p.start_seq, p.agent_index, p.restore),
            (13, 1, snap("snap_b"))
        );
    }

    /// Forking at `at` with the change `c` restarts the resolver from its first start.
    fn assert_moves_back_to_resolver_start(t: &Team, c: &HashMap<String, Change>, at: i64) {
        let p = plan_fork(&events(), t, at, c).unwrap();
        assert_eq!(
            (p.start_seq, p.moved_from, p.agent_index),
            (8, Some(at), 1),
            "at {at}"
        );
        assert_eq!(p.reason.as_deref(), Some(REASON_EARLIER));
        assert_eq!((p.resume, p.restore), (None, base()));
        assert_eq!(p.notes, vec!["N1".to_string()]);
    }

    #[test]
    fn change_to_finished_active_agent_moves_back() {
        let t = team(AgentKind::Open);
        let c = change(
            "resolver",
            Change {
                prompt: Some("New.".into()),
                ..Default::default()
            },
        );
        // At the Check (after the resolver's AgentEnd) and at its AgentEnd itself.
        for at in [15, 14] {
            assert_moves_back_to_resolver_start(&t, &c, at);
        }
        // Changes to both: the earliest finished one wins.
        let mut both = c.clone();
        both.insert(
            "triage".into(),
            Change {
                model: Some("m".into()),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &t, 15, &both).unwrap();
        assert_eq!((p.start_seq, p.agent_index), (2, 0));
        // Before its AgentEnd the resolver is still active: a prompt change resumes.
        let p = plan_fork(&events(), &t, 13, &c).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (13, None));
    }

    #[test]
    fn apply_changes_model_rules() {
        let mut t = team(AgentKind::Open);
        t.agents[0].model = Some("old/model".into());
        let ap =
            |c: Change| apply_changes(&t, &change(&t.agents[0].id.clone(), c)).agents[0].clone();
        let to = |kind: AgentKind, model: Option<&str>| Change {
            kind: Some(kind),
            model: model.map(Into::into),
            ..Default::default()
        };
        // A mode change resets the model.
        let a = ap(to(AgentKind::Codex, None));
        assert_eq!((a.kind, a.model), (AgentKind::Codex, None));
        // An explicit model with the mode change is kept.
        let a = ap(to(AgentKind::Codex, Some("gpt-x")));
        assert_eq!(a.model.as_deref(), Some("gpt-x"));
        // Same kind keeps the model.
        let a = ap(to(AgentKind::Open, None));
        assert_eq!(a.model.as_deref(), Some("old/model"));
        // Empty or blank text clears.
        let model = |m: &str| {
            ap(Change {
                model: Some(m.into()),
                ..Default::default()
            })
            .model
        };
        assert_eq!(model(""), None);
        assert_eq!(model("  "), None);
        assert_eq!(model("new").as_deref(), Some("new"));
    }

    #[test]
    fn same_or_empty_model_is_not_a_change() {
        let mut t = team(AgentKind::Open);
        t.agents[0].model = Some("m".into());
        let id = t.agents[0].id.clone();
        let same = Change {
            model: Some("m".into()),
            ..Default::default()
        };
        assert!(!is_change(&same, &t.agents[0]));
        let empty = Change {
            model: Some("".into()),
            ..Default::default()
        };
        assert!(is_change(&empty, &t.agents[0]));
        t.agents[0].model = None;
        assert!(!is_change(&empty, &t.agents[0]));
        assert!(is_change(&same, &t.agents[0]));
        // And plan_fork ignores it.
        let c = change(&id, empty);
        let p = plan_fork(&events(), &t, 11, &c).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (11, None));
    }

    #[test]
    fn same_kind_is_not_a_mode_change() {
        let t = team(AgentKind::Open);
        let c = change(
            "resolver",
            Change {
                kind: Some(AgentKind::Open),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &t, 11, &c).unwrap();
        assert_eq!(
            (p.start_seq, p.moved_from, p.resume),
            (11, None, Some(msgs("m11")))
        );
        // Nor a change to a finished agent.
        let c = change(
            "triage",
            Change {
                kind: Some(AgentKind::Open),
                ..Default::default()
            },
        );
        let p = plan_fork(&events(), &t, 11, &c).unwrap();
        assert_eq!((p.start_seq, p.agent_index), (11, 1));
    }

    #[test]
    fn bad_inputs_are_errors() {
        let t = team(AgentKind::Open);
        assert!(matches!(
            plan_fork(&events(), &t, 99, &no_changes()),
            Err(ForkError::UnknownSeq)
        ));
        let c = change(
            "nobody",
            Change {
                prompt: Some("x".into()),
                ..Default::default()
            },
        );
        assert!(
            matches!(plan_fork(&events(), &t, 11, &c), Err(ForkError::UnknownAgent(a)) if a == "nobody")
        );
        let evs: Vec<Event> = events()
            .into_iter()
            .filter(|e| e.kind != EventKind::Handoff)
            .collect();
        assert!(
            matches!(plan_fork(&evs, &t, 11, &no_changes()), Err(ForkError::MissingHandoff(a)) if a == "triage")
        );
    }

    #[test]
    fn errors_read_well() {
        assert_eq!(
            ForkError::BranchRunning.to_string(),
            "the branch is still running"
        );
        assert!(ForkError::UnknownAgent("bob".into())
            .to_string()
            .contains("bob"));
        assert!(ForkError::NoRestorePoint { seq: 12 }
            .to_string()
            .contains("12"));
        assert!(ForkError::MissingHandoff("triage".into())
            .to_string()
            .contains("triage"));
    }
}
