//! Driving a suite's runs. With a built-in runner arenad runs them one after another: it claims
//! the next slot, starts its run, waits for it to end, and goes on. With `external`, the external
//! system claims each next slot and arenad starts an external run for it, which the system then
//! drives through the external-run API.

use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use serde::Serialize;

use super::{Slot, Suite};
use crate::{
    app::AppState,
    external::{self, Started},
    model::BranchStatus,
    runner,
};

/// How often a built-in suite looks whether its current run has ended.
const POLL: Duration = Duration::from_millis(200);

/// Why a built-in suite stops when arenad shuts down.
const SHUTTING_DOWN: &str = "arenad stopped before the suite finished";

fn shutting_down(state: &AppState) -> bool {
    *state.shutdown.borrow()
}

/// Runs a built-in suite's runs one after another on a task of its own, then records that it
/// ended (with why, if it stopped early). On shutdown it stops; the next start records why.
pub fn spawn_builtin(state: &Arc<AppState>, suite: Suite) {
    let state = state.clone();
    tokio::spawn(async move {
        let stopped = drive_builtin(&state, &suite).await.err();
        if shutting_down(&state) {
            return;
        }
        let why = stopped.map(|e| format!("{e:#}"));
        if let Err(e) = state.store.end_suite(&suite.id, why.as_deref()).await {
            eprintln!("suite {}: could not record its end: {e:#}", suite.id);
        }
    });
}

/// Claims and runs each slot in turn; an error stops the suite.
async fn drive_builtin(state: &Arc<AppState>, suite: &Suite) -> Result<()> {
    loop {
        if shutting_down(state) {
            return Err(anyhow!(SHUTTING_DOWN));
        }
        let Some(slot) = state.store.claim_slot(&suite.id).await? else {
            return Ok(());
        };
        if let Some(branch) = start_builtin(state, suite, &slot).await? {
            wait_ended(state, &branch).await?;
        }
    }
}

/// Starts the slot's run (waiting out a cleanup): its branch, or None when its setup failed
/// (recorded on the slot). Saves over the limit stop the suite.
async fn start_builtin(
    state: &Arc<AppState>,
    suite: &Suite,
    slot: &Slot,
) -> Result<Option<String>> {
    let _cleanup = state.cleanup.clone().read_owned().await;
    if state.saves_over_limit().await? {
        let why = crate::api::SAVES_FULL;
        state.store.slot_failed(&suite.id, slot.ord, why).await?;
        return Err(anyhow!(why));
    }
    let (task, team) = (&slot.task_id, suite.team.clone());
    let scenario = suite.scenario_of(slot);
    match runner::start_run_in(state, task, team, suite.runner, scenario).await {
        Ok((run, branch)) => {
            state
                .store
                .fill_slot(&suite.id, slot.ord, &run, &branch)
                .await?;
            Ok(Some(branch))
        }
        Err(e) => {
            let why = format!("{e:#}");
            state.store.slot_failed(&suite.id, slot.ord, &why).await?;
            Ok(None)
        }
    }
}

/// Waits until the branch is no longer running (or arenad shuts down).
async fn wait_ended(state: &AppState, branch: &str) -> Result<()> {
    while !shutting_down(state) {
        match state.store.find_branch(branch).await? {
            Some(b) if b.status == BranchStatus::Running => tokio::time::sleep(POLL).await,
            _ => return Ok(()),
        }
    }
    Ok(())
}

/// The slot an external system claimed and the run arenad started for it.
#[derive(Debug, Clone, Serialize)]
pub struct ExternalClaim {
    pub task_id: String,
    /// Which of the suite's variants the run starts in (0 when it has none).
    pub variant: i32,
    pub repeat: i32,
    #[serde(flatten)]
    pub started: Started,
}

/// Claims the external suite's next slot and starts its run: None when every slot is claimed. A
/// run that could not start is recorded on its slot, and the error returned.
pub async fn claim_external(state: &AppState, suite: &Suite) -> Result<Option<ExternalClaim>> {
    let Some(slot) = state.store.claim_slot(&suite.id).await? else {
        return Ok(None);
    };
    let scenario = suite.scenario_of(&slot);
    match external::start(state, &slot.task_id, suite.team.clone(), scenario).await {
        Ok(started) => {
            let (run, branch) = (&started.run_id, &started.branch_id);
            state
                .store
                .fill_slot(&suite.id, slot.ord, run, branch)
                .await?;
            Ok(Some(ExternalClaim {
                task_id: slot.task_id,
                variant: slot.variant,
                repeat: slot.repeat,
                started,
            }))
        }
        Err(e) => {
            let why = format!("{e:#}");
            state.store.slot_failed(&suite.id, slot.ord, &why).await?;
            Err(e)
        }
    }
}
