use std::{sync::Arc, time::Instant};

use anyhow::{anyhow, Result};
use parts::proto::BenchDescribe;
use serde_json::{json, Value};

use crate::{
    agents::{AgentResult, AgentRun, Driver, FailureKind},
    app::AppState,
    exec::{mail_for, BranchCtx, ToolExec},
    model::*,
    overhead::{self, Cat},
    prompts,
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
/// records `RunStart`, and runs the team on a spawned task, one agent after another. A world
/// whose tools clash is refused before anything is recorded.
pub async fn start_run(
    state: &Arc<AppState>,
    task_id: &str,
    team: Team,
) -> Result<(RunId, BranchId)> {
    start_run_with(state, task_id, team, Runner::Chain).await
}

/// `start_run` with the agents run by `runner`.
pub async fn start_run_with(
    state: &Arc<AppState>,
    task_id: &str,
    team: Team,
    runner: Runner,
) -> Result<(RunId, BranchId)> {
    let (run_id, ctx) = begin_run(state, task_id, team, Rules::new(runner)).await?;
    let branch_id = ctx.branch_id.clone();
    spawn_branch(state, Arc::new(ctx), StartPoint::default());
    Ok((run_id, branch_id))
}

/// Creates a run and its first branch, starts its world from the base save and records
/// `RunStart` with `rules`: the branch, ready to be driven.
pub async fn begin_run(
    state: &AppState,
    task_id: &str,
    team: Team,
    rules: Rules,
) -> Result<(RunId, BranchCtx)> {
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
    let start = RunStartOf {
        branch_id: &branch_id,
        task_id,
        team: &team,
        rules,
    };
    let started = start_world(state, &bench, planned, start).await;
    let world = setup_or_infra_error(state, &branch_id, started).await?;
    let ctx = BranchCtx::new(branch_id, Arc::new(world), team, task_id.to_string());
    let ctx = ctx.with_rules(rules);
    set_up_task(state, &bench, &ctx).await?;
    Ok((run_id, ctx))
}

/// Makes the task's setup calls on a new run's world; when one fails, stops the world and ends
/// the branch `infra_error` with why.
async fn set_up_task(state: &AppState, bench: &BenchDescribe, ctx: &BranchCtx) -> Result<()> {
    let task = bench.tasks.iter().find(|t| t.id == ctx.task_id);
    let calls = task.map_or(&[][..], |t| &t.setup[..]);
    let Err(e) = ToolExec::new(state.store.clone()).setup(ctx, calls).await else {
        return Ok(());
    };
    ctx.world.stop().await;
    setup_failed(state, &ctx.branch_id, e.clone()).await;
    Err(anyhow!(e))
}

/// What a run's `RunStart` records besides its world.
struct RunStartOf<'a> {
    branch_id: &'a str,
    task_id: &'a str,
    team: &'a Team,
    rules: Rules,
}

/// A new branch's setup result; a failed setup ends the branch `infra_error` with its message.
pub async fn setup_or_infra_error<T>(state: &AppState, branch_id: &str, r: Result<T>) -> Result<T> {
    if let Err(e) = &r {
        setup_failed(state, branch_id, format!("{e:#}")).await;
    }
    r
}

/// Ends a branch whose setup failed `infra_error` with `error`; a store failure is logged.
pub async fn setup_failed(state: &AppState, branch_id: &str, error: String) {
    let set = state
        .store
        .set_status(branch_id, BranchStatus::InfraError, Some(error));
    if let Err(e) = set.await {
        eprintln!("branch {branch_id}: could not record its failed setup: {e:#}");
    }
}

/// Starts the first branch's world from the base save and records `RunStart`:
/// `{task_id, team, benchmark, world, seed, base_seed, fingerprints, saves, runner, handoff}`
/// (`saves` names only the parts that keep saves; `base_seed` is the seed the base save was made
/// with; `runner` and `handoff` are the run's rules).
async fn start_world(
    state: &AppState,
    bench: &BenchDescribe,
    planned: Plan,
    of: RunStartOf<'_>,
) -> Result<World> {
    let branch_id = of.branch_id;
    let seed = new_seed();
    let hub = state.hub.clone();
    let since = Instant::now();
    let world = World::start_planned(hub, bench, planned, branch_id, seed, WorldFrom::Base).await?;
    overhead::add_since(branch_id, Cat::Restore, since);
    let start = run_start(bench, &world, seed, &of);
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

/// The payload of `RunStart` (see `start_world`).
fn run_start(bench: &BenchDescribe, world: &World, seed: u64, of: &RunStartOf<'_>) -> Value {
    json!({
        "task_id": of.task_id,
        "team": of.team,
        "benchmark": bench.name,
        "world": bench.world,
        "seed": seed,
        "base_seed": world::SHARED_SEED,
        "fingerprints": world.fingerprints(),
        "saves": world.saves(),
        "runner": of.rules.runner,
        "handoff": of.rules.handoff_word(),
    })
}

/// Runs `run_branch` on its own task; a panic still ends the branch as `infra_error`. The branch
/// is live before this returns, so a cleanup can't slip in between the start and the task.
pub fn spawn_branch(state: &Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    let state = state.clone();
    state
        .live
        .write()
        .unwrap()
        .insert(ctx.branch_id.clone(), ctx.clone());
    tokio::spawn(async move {
        let branch_id = ctx.branch_id.clone();
        let inner = tokio::spawn(run_branch(state.clone(), ctx.clone(), from));
        if let Err(e) = inner.await {
            leave_live(&state, &ctx).await;
            let error = Some(format!("runner crashed: {e}"));
            record_ending(&state, &branch_id, (BranchStatus::InfraError, error)).await;
        }
    });
}

/// Runs the agents from `from.agent_index` of a branch already in `state.live` (the clock
/// ticking), scores the result, stops the world and sets the branch status. Never leaves the
/// branch `running`.
pub async fn run_branch(state: Arc<AppState>, ctx: Arc<BranchCtx>, from: StartPoint) {
    ctx.start_ticker(&state.store).await;
    let outcome = drive(&state, &ctx, from).await;
    conclude(&state, &ctx, outcome).await;
}

/// Makes a branch that an external system drives live, its clock ticking.
pub async fn go_live(state: &AppState, ctx: Arc<BranchCtx>) {
    let id = ctx.branch_id.clone();
    state.live.write().unwrap().insert(id, ctx.clone());
    ctx.start_ticker(&state.store).await;
}

/// Ends a live branch an external system drove: freezes and scores its world, stops it and
/// records how it ended. `None` when the branch is ending already.
pub async fn finish(state: &AppState, ctx: &BranchCtx) -> Option<Ending> {
    if !ctx.claim_end() {
        return None;
    }
    freeze(ctx).await;
    let outcome = check(state, ctx).await;
    Some(conclude(state, ctx, outcome).await)
}

/// Stops a branch's world and records how it ended: as `outcome` says, unless a world program
/// stopped, which makes it `infra_error` whatever the agents did after.
async fn conclude(state: &AppState, ctx: &BranchCtx, outcome: Result<Ending>) -> Ending {
    leave_live(state, ctx).await;
    let ending = match (ctx.fatal(), outcome) {
        (Some(f), _) => (BranchStatus::InfraError, Some(f)),
        (None, Ok(v)) => v,
        (None, Err(e)) => (BranchStatus::InfraError, Some(format!("{e:#}"))),
    };
    record_ending(state, &ctx.branch_id, ending.clone()).await;
    ending
}

/// Stops the branch's ticker and world, then takes it out of `state.live`. In that order: a
/// cleanup is refused while a branch is live, so it can't drop instances the world is stopping.
async fn leave_live(state: &AppState, ctx: &BranchCtx) {
    ctx.stop_ticker().await;
    ctx.world.stop().await;
    state.live.write().unwrap().remove(&ctx.branch_id);
}

/// How a branch ended: its status and error.
pub type Ending = (BranchStatus, Option<String>);

/// Records how a branch ended, trying once more when the store fails; a failure is logged (the
/// branch then stays `running` in the store).
async fn record_ending(state: &AppState, branch_id: &str, (status, error): Ending) {
    for attempt in 1..=2 {
        let set = state.store.set_status(branch_id, status, error.clone());
        match set.await {
            Ok(()) => return,
            Err(e) => {
                let what = format!("could not record its end ({status:?}), attempt {attempt}");
                eprintln!("branch {branch_id}: {what}: {e:#}");
            }
        }
    }
}

/// What every agent's input starts with: the task's ticket and the benchmark's instructions
/// under their title.
struct Brief {
    ticket: String,
    title: String,
    instructions: String,
}

async fn drive(state: &Arc<AppState>, ctx: &Arc<BranchCtx>, from: StartPoint) -> Result<Ending> {
    let brief = brief_of(state, ctx).await?;
    restore_notes(ctx, &from)?;
    let ended = match ctx.rules.runner {
        Runner::Chain => run_chain(state, ctx, &brief, from).await?,
        Runner::Concurrent => run_concurrent(state, ctx, &brief, from.agent_index).await?,
        Runner::External => return Err(anyhow!("an external system drives this run, not arenad")),
    };
    if let Some(end) = ended {
        return Ok(end);
    }
    freeze(ctx).await;
    check(state, ctx).await
}

/// The task's ticket and the benchmark's instructions.
async fn brief_of(state: &AppState, ctx: &BranchCtx) -> Result<Brief> {
    let bench = state.hub.bench_describe(ctx.world.bench()).await?;
    let task = bench.tasks.iter().find(|t| t.id == ctx.task_id);
    let ticket = task
        .map(|t| t.ticket.clone())
        .ok_or_else(|| anyhow!("unknown task {}", ctx.task_id))?;
    Ok(Brief {
        ticket,
        title: prompts::instructions_title(&bench).to_string(),
        instructions: bench.instructions,
    })
}

/// The agents from the start point on, one after another; `Some` when one ends the branch.
async fn run_chain(
    state: &Arc<AppState>,
    ctx: &Arc<BranchCtx>,
    brief: &Brief,
    from: StartPoint,
) -> Result<Option<Ending>> {
    let mut resume = from.resume;
    for idx in from.agent_index..ctx.team.agents.len() {
        if let Some(end) = run_agent(state, ctx, idx, brief, resume.take()).await? {
            return Ok(Some(end));
        }
    }
    Ok(None)
}

/// The agents from `first` on, all at once. Every one runs to its end; the branch then ends
/// with the worst of their endings, if any.
async fn run_concurrent(
    state: &Arc<AppState>,
    ctx: &Arc<BranchCtx>,
    brief: &Brief,
    first: usize,
) -> Result<Option<Ending>> {
    let runs = (first..ctx.team.agents.len()).map(|i| run_agent(state, ctx, i, brief, None));
    let mut worst = None;
    for ended in futures_util::future::join_all(runs).await {
        worst = worse(worst, ended?);
    }
    Ok(worst)
}

/// The worse of two endings: an infrastructure error before a failure, the first of equals.
fn worse(a: Option<Ending>, b: Option<Ending>) -> Option<Ending> {
    let infra = |e: &Option<Ending>| matches!(e, Some((BranchStatus::InfraError, _)));
    if infra(&a) || b.is_none() {
        return a;
    }
    if a.is_none() || infra(&b) {
        return b;
    }
    a
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

/// The ticket, the instructions, and the handoff notes left for the agent: in a legacy run the
/// previous agent's (which must exist), otherwise every note recorded so far for it.
async fn agent_input(
    state: &AppState,
    ctx: &BranchCtx,
    idx: usize,
    brief: &Brief,
) -> Result<String> {
    let Brief {
        ticket,
        title,
        instructions,
    } = brief;
    let mut input = format!("{ticket}\n# {title}\n\n{instructions}\n");
    let notes = match ctx.rules.handoff_required {
        true => legacy_note(ctx, idx)?.into_iter().collect(),
        false => notes_for(state, ctx, &ctx.team.agents[idx].id).await?,
    };
    for (from, note) in notes {
        input.push_str(&format!("\n# Handoff note from {from}\n\n{note}\n"));
    }
    Ok(input)
}

/// The previous agent's name and note, in a legacy run (none for the first agent).
fn legacy_note(ctx: &BranchCtx, idx: usize) -> Result<Option<(String, String)>> {
    let Some(prev) = idx.checked_sub(1).map(|i| &ctx.team.agents[i]) else {
        return Ok(None);
    };
    let spec = &ctx.team.agents[idx];
    let note = ctx
        .handoff_note(&prev.id)
        .ok_or_else(|| anyhow!("no handoff note from {} to start {}", prev.id, spec.id))?;
    Ok(Some((prev.name.clone(), note)))
}

/// The name of the sender and the note of each handoff recorded so far for `agent`.
async fn notes_for(
    state: &AppState,
    ctx: &BranchCtx,
    agent: &str,
) -> Result<Vec<(String, String)>> {
    let events = state.store.events(&ctx.branch_id).await?;
    let name_of = |id: &str| {
        let spec = ctx.team.agents.iter().find(|a| a.id == id);
        spec.map_or_else(|| id.to_string(), |a| a.name.clone())
    };
    let mail = mail_for(&events, agent);
    let notes = mail.iter().filter(|m| m["kind"] == "handoff").map(|m| {
        let from = name_of(m["from"].as_str().unwrap_or(""));
        (from, m["text"].as_str().unwrap_or("").to_string())
    });
    Ok(notes.collect())
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
    let input = agent_input(state, ctx, idx, brief).await?;
    let driver = driver_for(state, spec.kind)?;
    let mut start = json!({"name": spec.name, "kind": spec.kind, "model": spec.model});
    if let Some(p) = &spec.provider {
        start["provider"] = json!(p);
    }
    let id = Some(spec.id.as_str());
    ctx.append(&state.store, id, EventKind::AgentStart, start)
        .await?;
    let run = AgentRun {
        branch: ctx.clone(),
        spec: spec.clone(),
        input,
        resume,
    };
    let result = driver.run(run).await;
    record_end(state, ctx, &spec.id, &result).await?;
    Ok(agent_ending(ctx, &spec.id, result))
}

/// Records an agent's AgentEnd; in a legacy run the world is frozen first after the last agent,
/// so nothing happens in it after its end. Otherwise it freezes once every agent is done.
async fn record_end(
    state: &AppState,
    ctx: &BranchCtx,
    agent: &str,
    result: &AgentResult,
) -> Result<()> {
    if ctx.rules.handoff_required && ctx.is_last(agent) {
        freeze(ctx).await;
    }
    let end = serde_json::to_value(agent_end(result))?;
    ctx.append(&state.store, Some(agent), EventKind::AgentEnd, end)
        .await?;
    Ok(())
}

/// Once the agents are done the world stands still: no tick and no clock while it is scored.
/// Freezing twice is freezing once.
async fn freeze(ctx: &BranchCtx) {
    ctx.stop_ticker().await;
    ctx.clock.freeze();
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
        prompt_tokens: result.tokens["prompt_tokens"].as_i64(),
        completion_tokens: result.tokens["completion_tokens"].as_i64(),
        cost_usd: result.cost_usd,
        cost_source: result.cost_source.clone(),
        seconds: Some(result.seconds),
    }
}

/// How the branch ends after an agent, if it does: a stopped world program, a failed agent, or
/// (in a legacy run) a missing handoff.
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
    let required = ctx.rules.handoff_required && !ctx.is_last(agent);
    if required && ctx.handoff_note(agent).is_none() {
        return Some((BranchStatus::Failed, Some("no handoff".to_string())));
    }
    None
}

/// Scores the world and records the Check.
async fn check(state: &AppState, ctx: &BranchCtx) -> Result<Ending> {
    let since = Instant::now();
    let check = score(state, ctx).await?;
    overhead::add_since(&ctx.branch_id, Cat::Score, since);
    let ending = verdict(&check, &ctx.world.part_names());
    let payload = serde_json::to_value(&check)?;
    ctx.append(&state.store, None, EventKind::Check, payload)
        .await?;
    Ok(ending)
}

pub const NOTHING_CHECKED: &str =
    "no part of this world can be checked against the task's expected result";

/// How a scored branch ends: passed when some part was checked and every checked part matches;
/// failed with `NOTHING_CHECKED` when no part could be checked.
fn verdict(check: &CheckPayload, parts: &[String]) -> Ending {
    if check.unchecked.len() >= parts.len() {
        return (BranchStatus::Failed, Some(NOTHING_CHECKED.to_string()));
    }
    match check.passed {
        true => (BranchStatus::Passed, None),
        false => (BranchStatus::Failed, None),
    }
}

/// Saves every part's final state and has the benchmark check the world's end state.
async fn score_by_checks(ctx: &BranchCtx) -> Result<CheckPayload> {
    // Behind the gate: no late tool call lands between the final save and the checks.
    let _gate = ctx.gate().await;
    let final_saves = ctx.world.save_all().await?;
    let checks = ctx.world.score(&ctx.task_id).await?;
    let failed = checks.iter().filter(|c| !c.passed);
    let failed: Vec<String> = failed.map(|c| c.name.clone()).collect();
    Ok(CheckPayload {
        passed: !checks.is_empty() && failed.is_empty(),
        gold_db: None,
        differing_rows: Some(failed),
        checks: Some(checks),
        final_saves: Some(final_saves),
        unchecked: vec![],
    })
}

/// Saves every part's final state and compares the world with the task's gold saves; parts not
/// in them are listed as unchecked. A benchmark that scores itself has no gold saves.
async fn score(state: &AppState, ctx: &BranchCtx) -> Result<CheckPayload> {
    let bench = state.hub.bench_describe(ctx.world.bench()).await?;
    if bench.scored {
        return score_by_checks(ctx).await;
    }
    let gold = world::ensure_gold(&state.hub, &bench, &ctx.task_id).await?;
    // Behind the gate: no late tool call lands between the final save and the diff.
    let _gate = ctx.gate().await;
    let final_saves = ctx.world.save_all().await?;
    let differing = ctx.world.diff_with(&gold).await?;
    let unchecked = ctx.world.unchecked_parts();
    let gold_db = ctx
        .world
        .part_names()
        .iter()
        .find_map(|p| gold.get(p))
        .cloned();
    Ok(CheckPayload {
        passed: differing.is_empty() && unchecked.len() < ctx.world.part_names().len(),
        gold_db,
        differing_rows: Some(differing),
        checks: None,
        final_saves: Some(final_saves),
        unchecked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(passed: bool, unchecked: &[&str]) -> CheckPayload {
        CheckPayload {
            passed,
            gold_db: None,
            differing_rows: Some(vec![]),
            checks: None,
            final_saves: None,
            unchecked: unchecked.iter().map(|p| p.to_string()).collect(),
        }
    }

    #[test]
    fn a_branch_passes_only_when_some_part_was_checked() {
        let parts = ["db".to_string(), "sim".to_string()];
        assert_eq!(
            verdict(&check(true, &["sim"]), &parts),
            (BranchStatus::Passed, None)
        );
        assert_eq!(
            verdict(&check(false, &[]), &parts),
            (BranchStatus::Failed, None)
        );
        let none = verdict(&check(false, &["db", "sim"]), &parts);
        assert_eq!(
            none,
            (BranchStatus::Failed, Some(NOTHING_CHECKED.to_string()))
        );
    }
}
