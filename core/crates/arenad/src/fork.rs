//! Fork: go back to step N of a branch, change components, and rerun from there with the
//! world restored exactly as it was just before step N.

use std::{collections::HashMap, sync::Arc};

use serde::Serialize;
use serde_json::Value;

use crate::{
    app::AppState,
    exec::BranchCtx,
    model::*,
    recorded::{run_info, saves_at},
    runner::{self, StartPoint},
    world::{Saves, World, WorldFrom},
};

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

/// The model a change sets: `Some(None)` clears it (empty or blank text), `Some(Some(m))` sets it.
fn new_model(c: &Change) -> Option<Option<String>> {
    c.model.as_ref().map(|m| {
        let m = m.trim();
        (!m.is_empty()).then(|| m.to_string())
    })
}

/// True when `c` changes the agent's model (the same model, or clearing an unset one, is no change).
fn changes_model(c: &Change, spec: &AgentSpec) -> bool {
    new_model(c).is_some_and(|m| m != spec.model)
}

fn is_change(c: &Change, spec: &AgentSpec) -> bool {
    c.prompt.is_some() || changes_model(c, spec) || changes_kind(c, spec)
}

/// Plans a fork of a branch (its full event list, `team` its effective team) at `at_seq`.
/// Empty changes (no field set, or only the agent's current kind) are ignored.
pub fn plan_fork(
    events: &[Event],
    team: &Team,
    at_seq: i64,
    changes: &HashMap<String, Change>,
) -> Result<ForkPlan, ForkError> {
    if team.agents.is_empty() {
        return Err(ForkError::UnknownAgent(String::new()));
    }
    let index_of = |id: &str| team.agents.iter().position(|a| a.id == id);
    let mut changed: Vec<usize> = vec![];
    for (id, c) in changes {
        let i = index_of(id).ok_or_else(|| ForkError::UnknownAgent(id.clone()))?;
        if is_change(c, &team.agents[i]) {
            changed.push(i);
        }
    }
    let at = events
        .iter()
        .find(|e| e.seq == at_seq)
        .ok_or(ForkError::UnknownSeq)?;
    // An agent's real start: its first AgentStart (a resumed agent has a second one).
    let first_start = |i: usize| {
        events
            .iter()
            .find(|e| {
                e.kind == EventKind::AgentStart
                    && e.agent.as_deref() == Some(team.agents[i].id.as_str())
            })
            .map(|e| e.seq)
            .ok_or(ForkError::UnknownSeq)
    };

    // 1. The agent active at `at_seq`.
    let last_start = events
        .iter()
        .rev()
        .find(|e| e.seq <= at_seq && e.kind == EventKind::AgentStart);
    let mut a = match (at.kind, last_start) {
        (EventKind::RunStart, _) | (_, None) => 0,
        (_, Some(s)) => {
            let id = s.agent.clone().unwrap_or_default();
            index_of(&id).ok_or(ForkError::UnknownAgent(id))?
        }
    };

    // An agent has finished at `at_seq` once its AgentEnd is at or before it.
    let finished = |i: usize| {
        events.iter().any(|e| {
            e.seq <= at_seq
                && e.kind == EventKind::AgentEnd
                && e.agent.as_deref() == Some(team.agents[i].id.as_str())
        })
    };

    let mut resume = None;
    let earliest_finished_change = changed
        .iter()
        .copied()
        .filter(|&i| i < a || finished(i))
        .min();
    let (start_seq, reason): (i64, Option<&str>) = if let Some(e) = earliest_finished_change {
        // 2. A change to an agent that has finished (including `a`): restart from it.
        a = e;
        (first_start(a)?, Some(REASON_EARLIER))
    } else if changes
        .get(&team.agents[a].id)
        .is_some_and(|c| changes_kind(c, &team.agents[a]))
    {
        // 3. Mode change, or a CLI agent: restart the agent.
        (first_start(a)?, Some(REASON_KIND))
    } else if team.agents[a].kind != AgentKind::Open {
        (first_start(a)?, Some(REASON_CLI))
    } else if at.kind == EventKind::RunStart {
        // The run's start: begin with the first agent (or right after RunStart if it never began).
        (first_start(0).unwrap_or(at_seq + 1), Some(REASON_RUN_START))
    } else if at.kind == EventKind::AgentStart {
        (first_start(a)?, Some(REASON_AGENT_RESTART))
    } else {
        // 4. Open agent: resume from a model call.
        let agent_start = first_start(a)?;
        let model_call = if at.kind == EventKind::ModelCall {
            Some(at)
        } else {
            let id = team.agents[a].id.as_str();
            events.iter().rev().find(|e| {
                e.seq <= at_seq
                    && e.seq > agent_start
                    && e.kind == EventKind::ModelCall
                    && e.agent.as_deref() == Some(id)
            })
        };
        let messages =
            model_call.and_then(|m| m.payload["request"]["messages"].as_array().cloned());
        match (model_call, messages) {
            (Some(m), Some(msgs)) => {
                resume = Some(msgs);
                let why = match at.kind {
                    EventKind::ModelCall => None,
                    EventKind::ToolCall | EventKind::Handoff => Some(REASON_TOOL),
                    _ => Some(REASON_LAST_MODEL_CALL),
                };
                (m.seq, why)
            }
            _ => (agent_start, Some(REASON_NO_MODEL_CALL)),
        }
    };
    if start_seq == first_start(a).unwrap_or(i64::MIN) {
        resume = None;
    }

    // 5. The world just before `start_seq`: per part, the save of its last write before it.
    let restore = saves_at(events, start_seq - 1, &run_info(events).base)
        .map_err(|n| ForkError::NoRestorePoint { seq: n.seq })?;

    // 6. The recorded handoff notes of the agents before `a`.
    let notes = team.agents[..a]
        .iter()
        .map(|spec| {
            events
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
        .collect::<Result<Vec<_>, _>>()?;

    // 7.
    let moved = start_seq != at_seq;
    Ok(ForkPlan {
        start_seq,
        moved_from: moved.then_some(at_seq),
        reason: if moved {
            reason.map(str::to_string)
        } else {
            None
        },
        agent_index: a,
        restore,
        resume,
        notes,
    })
}

/// `team` with each agent's prompt/model/kind replaced where `changes` sets them.
pub fn apply_changes(team: &Team, changes: &HashMap<String, Change>) -> Team {
    let mut t = team.clone();
    for spec in &mut t.agents {
        if let Some(c) = changes.get(&spec.id) {
            if let Some(p) = &c.prompt {
                spec.prompt = p.clone();
            }
            if changes_kind(c, spec) {
                // The old model belongs to the old driver: the new driver's default applies.
                spec.model = None;
            }
            if let Some(m) = new_model(c) {
                spec.model = m;
            }
            if let Some(k) = c.kind {
                spec.kind = k;
            }
        }
    }
    t
}

/// Forks `branch_id` at `at_seq` with `changes` and starts the new branch running.
pub async fn fork(
    state: &Arc<AppState>,
    branch_id: &str,
    at_seq: i64,
    changes: HashMap<String, Change>,
) -> Result<ForkResponse, ForkError> {
    let parent = finished_branch(state, branch_id).await?;
    let events = state.store.events(branch_id).await?;
    let plan = plan_fork(&events, &parent.team, at_seq, &changes)?;
    let task_id = task_of(state, &events, &parent).await?;
    let team = apply_changes(&parent.team, &changes);
    let new_id = create_child(state, &parent, &plan, &team, &changes).await?;
    let started = start_world(state, &events, branch_id, &new_id, &plan).await;
    let world = runner::setup_or_infra_error(state, &new_id, started)
        .await
        .map_err(ForkError::Internal)?;
    let ctx = BranchCtx::new(new_id, Arc::new(world), team, task_id);
    Ok(spawn_child(state, ctx, plan))
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

/// Records the new branch: the parent's run, the fork point, the effective team and the changes.
async fn create_child(
    state: &AppState,
    parent: &Branch,
    plan: &ForkPlan,
    team: &Team,
    changes: &HashMap<String, Change>,
) -> anyhow::Result<String> {
    let changes_json = serde_json::to_value(changes)?;
    let from = Some((parent.id.as_str(), plan.start_seq));
    state
        .store
        .create_branch(
            &parent.run_id,
            from,
            team,
            &changes_json,
            plan.reason.clone(),
        )
        .await
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

/// Copies the parent's events before the fork point and starts the new branch's world from the
/// plan's saves, with the run's benchmark and seed.
async fn start_world(
    state: &AppState,
    events: &[Event],
    parent: &str,
    new_id: &str,
    plan: &ForkPlan,
) -> anyhow::Result<World> {
    state
        .store
        .copy_prefix(parent, new_id, plan.start_seq)
        .await?;
    let run = run_info(events);
    let bench = state.hub.bench_describe(&run.benchmark).await?;
    World::start(
        state.hub.clone(),
        &bench,
        new_id,
        run.seed,
        WorldFrom::exact(plan.restore.clone()),
    )
    .await
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

    #[test]
    fn fork_of_a_fork_with_two_agent_starts() {
        // Branch forked at seq 11 (resume): prefix 1..=10, then the resumed resolver.
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
        // AgentEnd and Check: from the agent's last model call.
        let p = plan_fork(&events(), &t, 14, &no_changes()).unwrap();
        assert_eq!((p.start_seq, p.moved_from), (13, Some(14)));
        let p = plan_fork(&events(), &t, 15, &no_changes()).unwrap();
        assert_eq!(
            (p.start_seq, p.agent_index, p.restore),
            (13, 1, snap("snap_b"))
        );
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
            let p = plan_fork(&events(), &t, at, &c).unwrap();
            assert_eq!(
                (p.start_seq, p.moved_from, p.agent_index),
                (8, Some(at), 1),
                "at {at}"
            );
            assert_eq!(p.reason.as_deref(), Some(REASON_EARLIER));
            assert_eq!((p.resume, p.restore), (None, base()));
            assert_eq!(p.notes, vec!["N1".to_string()]);
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
        // A mode change resets the model.
        let a = ap(Change {
            kind: Some(AgentKind::Codex),
            ..Default::default()
        });
        assert_eq!((a.kind, a.model), (AgentKind::Codex, None));
        // An explicit model with the mode change is kept.
        let a = ap(Change {
            kind: Some(AgentKind::Codex),
            model: Some("gpt-x".into()),
            ..Default::default()
        });
        assert_eq!(a.model.as_deref(), Some("gpt-x"));
        // Same kind keeps the model.
        let a = ap(Change {
            kind: Some(AgentKind::Open),
            ..Default::default()
        });
        assert_eq!(a.model.as_deref(), Some("old/model"));
        // Empty or blank text clears.
        assert_eq!(
            ap(Change {
                model: Some("".into()),
                ..Default::default()
            })
            .model,
            None
        );
        assert_eq!(
            ap(Change {
                model: Some("  ".into()),
                ..Default::default()
            })
            .model,
            None
        );
        assert_eq!(
            ap(Change {
                model: Some("new".into()),
                ..Default::default()
            })
            .model
            .as_deref(),
            Some("new")
        );
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
