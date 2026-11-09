//! Driving an experiment: one run at a time, in `run_order`, on a task of its own. Before each
//! run it looks whether the experiment was stopped or arenad is shutting down; a suite ends when
//! its last run did, the experiment when every run did.

use std::sync::Arc;

use anyhow::Result;

use super::{run_order, STOPPED};
use crate::{
    app::AppState,
    runner,
    suite::{self, shutting_down, Suite},
};

/// Drives the experiment `id` over its `suites` (in setup order) on a task of its own, then
/// records that it ended (with why, if it stopped early). On shutdown it stops; the next start
/// records why.
pub fn spawn(state: &Arc<AppState>, id: String, suites: Vec<Suite>) {
    let state = state.clone();
    tokio::spawn(async move {
        let stopped = drive(&state, &id, &suites).await.err();
        if shutting_down(&state) {
            return;
        }
        let why = stopped.map(|e| format!("{e:#}"));
        if let Err(e) = state.store.end_experiment(&id, why.as_deref()).await {
            eprintln!("experiment {id}: could not record its end: {e:#}");
        }
    });
}

/// Makes each run in turn until all are done, the experiment is stopped or arenad shuts down;
/// an error stops the experiment.
async fn drive(state: &Arc<AppState>, id: &str, suites: &[Suite]) -> Result<()> {
    let runs: Vec<i32> = suites.iter().map(|s| s.repeats).collect();
    for (setup, run) in run_order(&runs) {
        if shutting_down(state) || state.store.experiment_ended(id).await? {
            return Ok(());
        }
        let suite = &suites[setup];
        next_run(state, id, suite).await?;
        if run + 1 == suite.repeats {
            state.store.end_suite(&suite.id, None).await?;
        }
    }
    Ok(())
}

/// Claims the suite's next slot, starts its run and waits for its end. A run that started just
/// as the experiment was stopped is stopped too.
async fn next_run(state: &Arc<AppState>, id: &str, suite: &Suite) -> Result<()> {
    let Some(slot) = state.store.claim_slot(&suite.id).await? else {
        return Ok(());
    };
    let Some(branch) = suite::start_builtin(state, suite, &slot).await? else {
        return Ok(());
    };
    if state.store.experiment_ended(id).await? {
        stop_branch(state, &branch).await;
    }
    suite::wait_ended(state, &branch).await
}

/// Stops the branch if it is live here (as `POST /api/branches/{id}/stop` does).
async fn stop_branch(state: &AppState, branch: &str) {
    let ctx = state.live.read().unwrap().get(branch).cloned();
    if let Some(ctx) = ctx {
        runner::stop(state, &ctx).await;
    }
}

/// Stops the experiment: no further run starts, its suites end, and the run that is running now
/// is stopped. Stopping one that ended changes nothing.
pub async fn stop(state: &AppState, id: &str) -> Result<()> {
    state.store.end_experiment(id, Some(STOPPED)).await?;
    for s in state.store.experiment_suites(id).await? {
        for slot in state.store.slots(&s.id).await? {
            if let Some(branch) = &slot.branch_id {
                stop_branch(state, branch).await;
            }
        }
    }
    Ok(())
}
