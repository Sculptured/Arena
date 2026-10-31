use std::sync::Arc;

use anyhow::{anyhow, Result};
use parts::proto::BenchDescribe;
use serde_json::{json, Value};

use crate::{
    agents::{AgentResult, AgentRun, Driver, FailureKind},
    app::AppState,
    exec::BranchCtx,
    model::*,
    store::{BranchId, RunId},
    world::{self, Plan, World, WorldFrom},
};

/// Where a branch's work starts. `notes[j]` is the recorded handoff note of agent `j`
/// (one per agent before `agent_index`); `resume` applies to the agent at `agent_index`.
#[derive(Debug, Clone, Default)]
pub struct StartPoint {
    pub agent_index: usize,
    pub resume: Option<Vec<Value>>,
    pub notes: Vec<String>,
}

/// A new branch seed: random, below 2^53 so every JSON reader keeps it exact.
fn new_seed() -> u64 {
    (uuid::Uuid::new_v4().as_u128() as u64) & ((1 << 53) - 1)
}

/// The benchmark has no task with this id.
#[derive(Debug)]
pub struct UnknownTask(pub String);

impl std::fmt::Display for UnknownTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown task {}", self.0)
    }
}

impl std::error::Error for UnknownTask {}

/// Creates a run and its first branch (its world started from the benchmark's base save),
/// records `RunStart`, and runs the team on a spawned task. A world whose tools clash is
/// refused before anything is recorded.
pub async fn start_run(
    state: &Arc<AppState>,
    task_id: &str,
    team: Team,
) -> Result<(RunId, BranchId)> {
    let bench = state.bench().await?;
    if !bench.tasks.iter().any(|t| t.id == task_id) {
        return Err(UnknownTask(task_id.to_string()).into());
    }
    let planned = world::plan(&state.hub, &bench).await?;
    let run_id = state.store.create_run(task_id, &team).await?;
    let branch_id = state
        .store
        .create_branch(&run_id, None, &team, &json!([]), None)
        .await?;
    let started = start_world(state, &bench, planned, &branch_id, task_id, &team).await;
    let world = setup_or_infra_error(state, &branch_id, started).await?;
    let ctx = Arc::new(BranchCtx::new(
        branch_id.clone(),
        Arc::new(world),
        team,
        task_id.to_string(),
    ));
    spawn_branch(state, ctx, StartPoint::default());
    Ok((run_id, branch_id))
}

/// A new branch's setup result; a failed setup ends the branch `infra_error` with its message.
pub async fn setup_or_infra_error<T>(state: &AppState, branch_id: &str, r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        let error = Some(format!("{e:#}"));
        let _ = state
            .store
            .set_status(branch_id, BranchStatus::InfraError, error)
            .await;
    }
    r
}

/// Starts the first branch's world from the base save and records `RunStart`:
/// `{task_id, team, benchmark, world, seed, fingerprints, saves}`.
async fn start_world(
    state: &AppState,
    bench: &BenchDescribe,
    planned: Plan,
    branch_id: &str,
    task_id: &str,
    team: &Team,
) -> Result<World> {
    let seed = new_seed();
    let hub = state.hub.clone();
    let world = World::start_planned(hub, bench, planned, branch_id, seed, WorldFrom::Base).await?;
    let start = json!({
        "task_id": task_id,
        "team": team,
        "benchmark": bench.name,
        "world": bench.world,
        "seed": seed,
        "fingerprints": world.fingerprints(),
        "saves": world.saves(),
    });
    if let Err(e) = state
        .store
        .append(branch_id, None, EventKind::RunStart, start)
        .await
    {
        world.stop().await;
        return Err(e);
    }
    Ok(world)
}

/// Runs `run_branch` on its own task; a panic still ends the branch as `infra_error`.
pub fn spawn_branch(state: &Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    let state = state.clone();
    tokio::spawn(async move {
        let branch_id = ctx.branch_id.clone();
        let world = ctx.world.clone();
        let inner = tokio::spawn(run_branch(state.clone(), ctx, from));
        if let Err(e) = inner.await {
            state.live.write().unwrap().remove(&branch_id);
            world.stop().await;
            let _ = state
                .store
                .set_status(
                    &branch_id,
                    BranchStatus::InfraError,
                    Some(format!("runner crashed: {e}")),
                )
                .await;
        }
    });
}

/// Runs the agents from `from.agent_index`, scores the result, stops the world and sets the
/// branch status. Never leaves the branch `running`.
pub async fn run_branch(state: Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    state
        .live
        .write()
        .unwrap()
        .insert(ctx.branch_id.clone(), ctx.clone());
    let outcome = drive(&state, &ctx, from).await;
    state.live.write().unwrap().remove(&ctx.branch_id);
    ctx.world.stop().await;
    let (status, error) = match (ctx.fatal(), outcome) {
        // A world program stopped: whatever the agents did after it doesn't count.
        (Some(f), _) => (BranchStatus::InfraError, Some(f)),
        (None, Ok(v)) => v,
        (None, Err(e)) => (BranchStatus::InfraError, Some(format!("{e:#}"))),
    };
    let _ = state.store.set_status(&ctx.branch_id, status, error).await;
}

/// How a branch ended: its status and error.
type Ending = (BranchStatus, Option<String>);

/// What every agent's input starts with: the task's ticket and the benchmark's instructions.
struct Brief {
    ticket: String,
    policy: String,
}

async fn drive(state: &Arc<AppState>, ctx: &Arc<BranchCtx>, from: StartPoint) -> Result<Ending> {
    let bench = state.hub.bench_describe(ctx.world.bench()).await?;
    let task = bench.tasks.iter().find(|t| t.id == ctx.task_id);
    let ticket = task
        .map(|t| t.ticket.clone())
        .ok_or_else(|| anyhow!("unknown task {}", ctx.task_id))?;
    let brief = Brief {
        ticket,
        policy: bench.instructions,
    };
    restore_notes(ctx, &from)?;
    let mut resume = from.resume;
    for idx in from.agent_index..ctx.team.agents.len() {
        if let Some(end) = run_agent(state, ctx, idx, &brief, resume.take()).await? {
            return Ok(end);
        }
    }
    check(state, ctx).await
}

/// Carries over the recorded handoff notes of the agents before the start point.
fn restore_notes(ctx: &BranchCtx, from: &StartPoint) -> Result<()> {
    let agents = &ctx.team.agents;
    if from.agent_index >= agents.len() {
        return Err(anyhow!(
            "start agent index {} out of range",
            from.agent_index
        ));
    }
    let mut h = ctx.handoffs.lock().unwrap();
    for (spec, note) in agents.iter().zip(&from.notes).take(from.agent_index) {
        h.insert(spec.id.clone(), note.clone());
    }
    Ok(())
}

/// The ticket, the policy, and the previous agent's handoff note.
fn agent_input(ctx: &BranchCtx, idx: usize, brief: &Brief) -> Result<String> {
    let Brief { ticket, policy } = brief;
    let mut input = format!("{ticket}\n# Store policy\n\n{policy}\n");
    if let Some(prev) = idx.checked_sub(1).map(|i| &ctx.team.agents[i]) {
        let spec = &ctx.team.agents[idx];
        let note = ctx
            .handoff_note(&prev.id)
            .ok_or_else(|| anyhow!("no handoff note from {} to start {}", prev.id, spec.id))?;
        input.push_str(&format!("\n# Handoff note from {}\n\n{note}\n", prev.name));
    }
    Ok(input)
}

/// Runs agent `idx` between its AgentStart and AgentEnd events; `Some` when the branch ends here.
async fn run_agent(
    state: &Arc<AppState>,
    ctx: &Arc<BranchCtx>,
    idx: usize,
    brief: &Brief,
    resume: Option<Vec<Value>>,
) -> Result<Option<Ending>> {
    let spec = ctx.team.agents[idx].clone();
    let input = agent_input(ctx, idx, brief)?;
    let driver = driver_for(state, spec.kind)?;
    let start = json!({"name": spec.name, "kind": spec.kind, "model": spec.model});
    let id = Some(spec.id.as_str());
    state
        .store
        .append(&ctx.branch_id, id, EventKind::AgentStart, start)
        .await?;
    let run = AgentRun {
        branch: ctx.clone(),
        spec: spec.clone(),
        input,
        resume,
    };
    let result = driver.run(run).await;
    let end = serde_json::to_value(agent_end(&result))?;
    state
        .store
        .append(&ctx.branch_id, id, EventKind::AgentEnd, end)
        .await?;
    Ok(agent_ending(ctx, &spec.id, result))
}

fn driver_for(state: &AppState, kind: AgentKind) -> Result<Arc<dyn Driver>> {
    let drivers = state
        .drivers
        .get()
        .ok_or_else(|| anyhow!("no drivers installed"))?;
    drivers
        .get(&kind)
        .cloned()
        .ok_or_else(|| anyhow!("no driver for agent kind {kind:?}"))
}

fn agent_end(result: &AgentResult) -> AgentEndPayload {
    AgentEndPayload {
        ok: result.ok,
        error: result.error.clone(),
        tokens: result.tokens["total_tokens"].as_i64(),
        cost_usd: result.cost_usd,
        cost_source: result.cost_source.clone(),
        seconds: Some(result.seconds),
    }
}

/// How the branch ends after an agent, if it does: a stopped world program, a failed agent, or
/// a missing handoff.
fn agent_ending(ctx: &BranchCtx, agent: &str, result: AgentResult) -> Option<Ending> {
    if let Some(f) = ctx.fatal() {
        return Some((BranchStatus::InfraError, Some(f)));
    }
    if !result.ok {
        let e = result.error.unwrap_or_else(|| "agent failed".to_string());
        // The model's own behavior is a failed attempt, not a broken harness.
        let failed = result.failure == Some(FailureKind::Model);
        return Some((
            if failed {
                BranchStatus::Failed
            } else {
                BranchStatus::InfraError
            },
            Some(e),
        ));
    }
    if !ctx.is_last(agent) && ctx.handoff_note(agent).is_none() {
        return Some((BranchStatus::Failed, Some("no handoff".to_string())));
    }
    None
}

/// Scores the world and records the Check.
async fn check(state: &Arc<AppState>, ctx: &BranchCtx) -> Result<Ending> {
    let check = score(state, ctx).await?;
    let status = if check.passed {
        BranchStatus::Passed
    } else {
        BranchStatus::Failed
    };
    state
        .store
        .append(
            &ctx.branch_id,
            None,
            EventKind::Check,
            serde_json::to_value(&check)?,
        )
        .await?;
    Ok((status, None))
}

/// Saves every part's final state and compares the world with the task's gold saves.
async fn score(state: &Arc<AppState>, ctx: &BranchCtx) -> Result<CheckPayload> {
    let bench = state.hub.bench_describe(ctx.world.bench()).await?;
    let gold = world::ensure_gold(&state.hub, &bench, &ctx.task_id).await?;
    // Behind the gate: no late tool call lands between the final save and the diff.
    let _gate = ctx.gate().await;
    let final_saves = ctx.world.save_all().await?;
    let differing = ctx.world.diff_with(&gold).await?;
    let gold_db = ctx
        .world
        .part_names()
        .first()
        .and_then(|p| gold.get(p))
        .cloned();
    Ok(CheckPayload {
        passed: differing.is_empty(),
        gold_db,
        differing_rows: Some(differing),
        final_saves: Some(final_saves),
    })
}

/// Keys `<table>/<id>` whose documents differ between the dumps (present on one side only counts).
pub fn differing_rows(a: &Value, b: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let mut out = vec![];
    let mut tables: Vec<&String> = a
        .as_object()
        .into_iter()
        .chain(b.as_object())
        .flat_map(|m| m.keys())
        .collect();
    tables.sort();
    tables.dedup();
    for t in tables {
        let (ta, tb) = (
            a[t].as_object().unwrap_or(&empty),
            b[t].as_object().unwrap_or(&empty),
        );
        for id in ta.keys().chain(tb.keys()) {
            if ta.get(id) != tb.get(id) {
                out.push(format!("{t}/{id}"));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}
