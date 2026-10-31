//! The clock ticker of a live branch: one task that, about every 200 ms, advances the world's
//! clock parts to the branch clock (behind the branch gate, through `ToolExec::tick`), until the
//! branch stops it.

use std::{
    collections::HashSet,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use tokio::{sync::oneshot, task::JoinHandle, time::MissedTickBehavior};

use crate::exec::{BranchCtx, ToolExec};

/// How often a ticker advances the clock parts.
pub const TICK: Duration = Duration::from_millis(200);

/// The branches whose ticker task is running.
static RUNNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

/// True while `branch` has a ticker task running.
pub fn is_running(branch: &str) -> bool {
    RUNNING.lock().unwrap().contains(branch)
}

/// Marks a branch's ticker running until dropped (also when its task is cancelled).
struct Running(String);

impl Running {
    fn new(branch: &str) -> Running {
        RUNNING.lock().unwrap().insert(branch.to_string());
        Running(branch.to_string())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.lock().unwrap().remove(&self.0);
    }
}

/// A running ticker task.
pub struct Ticker {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Ticker {
    /// Starts ticking `ctx`'s world; None when the world has no clock parts.
    pub fn start(exec: ToolExec, ctx: Arc<BranchCtx>) -> Option<Ticker> {
        if !ctx.world.uses_clock() {
            return None;
        }
        let (stop, stopped) = oneshot::channel();
        let running = Running::new(&ctx.branch_id);
        let task = tokio::spawn(tick_until(exec, ctx, stopped, running));
        Some(Ticker { stop, task })
    }

    /// Stops the ticker after its current tick and waits for its task to end.
    pub async fn stop(self) {
        let _ = self.stop.send(());
        let _ = self.task.await;
    }
}

async fn tick_until(
    exec: ToolExec,
    ctx: Arc<BranchCtx>,
    mut stopped: oneshot::Receiver<()>,
    _running: Running,
) {
    let mut every = tokio::time::interval(TICK);
    every.set_missed_tick_behavior(MissedTickBehavior::Delay);
    every.tick().await; // the first tick is immediate
    let mut last_error = None;
    loop {
        tokio::select! {
            biased; // a stop wins over a tick that is due
            _ = &mut stopped => return,
            _ = every.tick() => {
                let failed = exec.tick(&ctx).await.err();
                log_new_error(&ctx.branch_id, &mut last_error, failed);
            }
        }
    }
}

/// Logs a tick failure unless it repeats the last one (a failing part would otherwise log every
/// tick); a tick that works forgets the last failure.
fn log_new_error(branch: &str, last: &mut Option<String>, failed: Option<String>) {
    if failed.is_some() && failed != *last {
        eprintln!(
            "branch {branch}: clock tick failed: {}",
            failed.as_deref().unwrap_or("")
        );
    }
    *last = failed;
}
