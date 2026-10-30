use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use world::snapshot::Source;

use crate::{
    agents::{AgentRun, FailureKind},
    app::AppState,
    exec::BranchCtx,
    model::*,
    store::{BranchId, RunId},
};

/// Where a branch's work starts. `notes[j]` is the recorded handoff note of agent `j`
/// (one per agent before `agent_index`); `resume` applies to the agent at `agent_index`.
#[derive(Debug, Clone, Default)]
pub struct StartPoint {
    pub agent_index: usize,
    pub resume: Option<Vec<Value>>,
    pub notes: Vec<String>,
}

/// Creates a run and its first branch (from `world_base`), records `RunStart`, and runs the
/// team on a spawned task.
pub async fn start_run(state: &Arc<AppState>, task_id: &str, team: Team) -> Result<(RunId, BranchId)> {
    if state.tasks()?.get(task_id).is_none() {
        return Err(anyhow!("unknown task {task_id}"));
    }
    let run_id = state.store.create_run(task_id, &team).await?;
    let branch_id = state.store.create_branch(&run_id, None, &team, &json!([]), None).await?;
    let setup = async {
        let db = state.worlds.new_branch(&branch_id, Source::Base).await?;
        state
            .store
            .append(&branch_id, None, EventKind::RunStart, json!({"task_id": task_id, "team": team}))
            .await?;
        Ok::<_, anyhow::Error>(db)
    };
    let db = match setup.await {
        Ok(db) => db,
        Err(e) => {
            let _ = state.store.set_status(&branch_id, BranchStatus::InfraError, Some(format!("{e:#}"))).await;
            return Err(e);
        }
    };
    let ctx = Arc::new(BranchCtx::new(branch_id.clone(), db, team, task_id.to_string()));
    spawn_branch(state, ctx, StartPoint::default());
    Ok((run_id, branch_id))
}

/// Runs `run_branch` on its own task; a panic still ends the branch as `infra_error`.
pub fn spawn_branch(state: &Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    let state = state.clone();
    tokio::spawn(async move {
        let branch_id = ctx.branch_id.clone();
        let inner = tokio::spawn(run_branch(state.clone(), ctx, from));
        if let Err(e) = inner.await {
            state.live.write().unwrap().remove(&branch_id);
            let _ = state.store.set_status(&branch_id, BranchStatus::InfraError, Some(format!("runner crashed: {e}"))).await;
        }
    });
}

/// Runs the agents from `from.agent_index`, scores the result and sets the branch status.
/// Never leaves the branch `running`.
pub async fn run_branch(state: Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    state.live.write().unwrap().insert(ctx.branch_id.clone(), ctx.clone());
    let outcome = drive(&state, &ctx, from).await;
    state.live.write().unwrap().remove(&ctx.branch_id);
    let (status, error) = match outcome {
        Ok(v) => v,
        Err(e) => (BranchStatus::InfraError, Some(format!("{e:#}"))),
    };
    let _ = state.store.set_status(&ctx.branch_id, status, error).await;
}

async fn drive(state: &Arc<AppState>, ctx: &Arc<BranchCtx>, from: StartPoint) -> Result<(BranchStatus, Option<String>)> {
    let tasks = state.tasks()?;
    let ticket = tasks.ticket(&ctx.task_id).ok_or_else(|| anyhow!("unknown task {}", ctx.task_id))?;
    let policy = std::fs::read_to_string(state.config.data_dir.join("policy.md"))?;
    let drivers = state.drivers.get().ok_or_else(|| anyhow!("no drivers installed"))?;
    let agents = &ctx.team.agents;
    if from.agent_index >= agents.len() {
        return Err(anyhow!("start agent index {} out of range", from.agent_index));
    }
    {
        let mut h = ctx.handoffs.lock().unwrap();
        for (spec, note) in agents.iter().zip(&from.notes).take(from.agent_index) {
            h.insert(spec.id.clone(), note.clone());
        }
    }
    let mut resume = from.resume;
    for idx in from.agent_index..agents.len() {
        let spec = agents[idx].clone();
        let mut input = format!("{ticket}\n# Store policy\n\n{policy}\n");
        if idx > 0 {
            let prev = &agents[idx - 1];
            let note = ctx
                .handoff_note(&prev.id)
                .ok_or_else(|| anyhow!("no handoff note from {} to start {}", prev.id, spec.id))?;
            input.push_str(&format!("\n# Handoff note from {}\n\n{note}\n", prev.name));
        }
        let driver = drivers
            .get(&spec.kind)
            .ok_or_else(|| anyhow!("no driver for agent kind {:?}", spec.kind))?
            .clone();
        let id = Some(spec.id.as_str());
        state
            .store
            .append(&ctx.branch_id, id, EventKind::AgentStart, json!({"name": spec.name, "kind": spec.kind, "model": spec.model}))
            .await?;
        let result = driver
            .run(AgentRun { branch: ctx.clone(), spec: spec.clone(), input, resume: resume.take() })
            .await;
        let end = AgentEndPayload {
            ok: result.ok,
            error: result.error.clone(),
            tokens: result.tokens["total_tokens"].as_i64(),
            cost_usd: result.cost_usd,
            cost_source: result.cost_source.clone(),
            seconds: Some(result.seconds),
        };
        state.store.append(&ctx.branch_id, id, EventKind::AgentEnd, serde_json::to_value(&end)?).await?;
        if !result.ok {
            let e = result.error.unwrap_or_else(|| "agent failed".to_string());
            // The model's own behavior is a failed attempt, not a broken harness.
            let status = if result.failure == Some(FailureKind::Model) { BranchStatus::Failed } else { BranchStatus::InfraError };
            return Ok((status, Some(e)));
        }
        if !ctx.is_last(&spec.id) && ctx.handoff_note(&spec.id).is_none() {
            return Ok((BranchStatus::Failed, Some("no handoff".to_string())));
        }
    }
    let check = score(state, ctx).await?;
    let passed = check.passed;
    state.store.append(&ctx.branch_id, None, EventKind::Check, serde_json::to_value(&check)?).await?;
    Ok((if passed { BranchStatus::Passed } else { BranchStatus::Failed }, None))
}

/// Compares the branch database with the task's gold database.
async fn score(state: &Arc<AppState>, ctx: &BranchCtx) -> Result<CheckPayload> {
    let task = state.tasks()?.get(&ctx.task_id).ok_or_else(|| anyhow!("unknown task {}", ctx.task_id))?.clone();
    let gold_name = format!("gold_{}", ctx.task_id);
    // Creating a copy from the gold source makes `Worlds` build the gold database under its
    // world_base lock if it is missing; the copy is then dumped and dropped.
    let copy = state.worlds.new_branch(&format!("goldcopy_{}", ctx.branch_id), Source::Gold(task)).await?;
    let gold = state.worlds.with_client(&copy, async |c| world::dump(c).await).await;
    let _ = state.worlds.drop_db(&copy).await;
    let gold = gold?;
    let branch = state.worlds.with_client(&ctx.db, async |c| world::dump(c).await).await?;
    let differing = differing_rows(&branch, &gold);
    Ok(CheckPayload { passed: differing.is_empty(), gold_db: Some(gold_name), differing_rows: Some(differing) })
}

/// Keys `<table>/<id>` whose documents differ between the dumps (present on one side only counts).
pub fn differing_rows(a: &Value, b: &Value) -> Vec<String> {
    let empty = serde_json::Map::new();
    let mut out = vec![];
    let mut tables: Vec<&String> = a.as_object().into_iter().chain(b.as_object()).flat_map(|m| m.keys()).collect();
    tables.sort();
    tables.dedup();
    for t in tables {
        let (ta, tb) = (a[t].as_object().unwrap_or(&empty), b[t].as_object().unwrap_or(&empty));
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
