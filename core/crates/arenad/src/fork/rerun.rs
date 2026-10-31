//! What a rerun restores and how: per part, from its save or by replay (the restore table, with
//! the restore asked for), the fork plan, the fingerprint check for replays, and starting the new
//! branch's world that way.

use std::collections::{BTreeMap, BTreeSet};

use parts::proto::{BenchDescribe, FromSave};
use serde::Serialize;

use super::{plan_fork_with, ForkError, ForkPlan};
use crate::{
    app::AppState,
    model::{Change, Event, RestoredHow, Team},
    recorded::{clock_at, run_info, RunInfo},
    replay::{self, ReplayError},
    restore::{restore_plan, How},
    world::{self, PartStart, Plan, StartSaves, StartingPart, World, WorldError, WorldFrom},
};

/// A fork request: where, what changes, and the restore asked for.
#[derive(Debug, Clone)]
pub struct ForkRequest {
    pub at_seq: i64,
    pub changes: std::collections::HashMap<String, Change>,
    pub restore: parts::proto::Mode,
}

/// How one part of the new world is restored.
pub struct PartRestore {
    pub part: String,
    pub how: How,
    /// The tools the part lists (its replayable inputs).
    pub tools: Vec<String>,
}

/// Everything a rerun needs before it starts: the run, its benchmark as it is now, the fork
/// plan, each part's restore (in world order) and the clock to start at.
pub struct Rerun {
    pub run: RunInfo,
    pub bench: BenchDescribe,
    pub plan: ForkPlan,
    pub parts: Vec<PartRestore>,
    pub clock_ms: i64,
}

impl Rerun {
    fn replayed(&self) -> impl Iterator<Item = &PartRestore> {
        self.parts.iter().filter(|p| p.how == How::Replay)
    }

    /// Each part and how it is restored, by part name.
    pub fn hows(&self) -> BTreeMap<String, How> {
        self.parts.iter().map(|p| (p.part.clone(), p.how)).collect()
    }
}

/// Each part's restore in `mode`, in world order; the first part that can't be is refused.
fn part_restores(world: &Plan, mode: parts::proto::Mode) -> Result<Vec<PartRestore>, ForkError> {
    let describes = world
        .parts
        .iter()
        .map(|p| (p.part.part.as_str(), &p.describe.restore));
    let hows = restore_plan(describes, mode).map_err(ForkError::Refused)?;
    Ok(world
        .parts
        .iter()
        .map(|p| PartRestore {
            part: p.part.part.clone(),
            how: hows[&p.part.part],
            tools: p.describe.tools.iter().map(|t| t.name.clone()).collect(),
        })
        .collect())
}

/// Plans a rerun of a branch (its events and team) for `req`.
pub async fn prepare(
    state: &AppState,
    events: &[Event],
    team: &Team,
    req: &ForkRequest,
) -> Result<Rerun, ForkError> {
    if !events.iter().any(|e| e.seq == req.at_seq) {
        return Err(ForkError::UnknownSeq);
    }
    let run = run_info(events);
    let bench = state.hub.bench_describe(&run.benchmark).await?;
    let world = world::plan(&state.hub, &bench).await?;
    let parts = part_restores(&world, req.restore)?;
    let replayed: BTreeSet<String> = parts
        .iter()
        .filter(|p| p.how == How::Replay)
        .map(|p| p.part.clone())
        .collect();
    let plan = plan_fork_with(events, team, req.at_seq, &req.changes, &replayed)?;
    check_fingerprints(&run, &world, &replayed)?;
    let clock_ms = clock_at(events, plan.start_seq);
    Ok(Rerun {
        run,
        bench,
        plan,
        parts,
        clock_ms,
    })
}

/// A replayed part whose integration's fingerprint differs from the one the run recorded is
/// refused (a part the run recorded none for is not checked).
fn check_fingerprints(
    run: &RunInfo,
    world: &Plan,
    replayed: &BTreeSet<String>,
) -> Result<(), ForkError> {
    let differs = |p: &&world::PlannedPart| {
        run.fingerprints
            .get(&p.part.part)
            .is_some_and(|f| *f != p.describe.fingerprint)
    };
    match world
        .parts
        .iter()
        .filter(|p| replayed.contains(&p.part.part))
        .find(differs)
    {
        Some(p) => Err(ForkError::FingerprintMismatch {
            part: p.part.part.clone(),
        }),
        None => Ok(()),
    }
}

/// How each part of the new world starts: from the plan's save, or fresh to be replayed.
fn start_saves(rerun: &Rerun) -> Result<StartSaves, ForkError> {
    let mut out = StartSaves::new();
    for p in &rerun.parts {
        let start = match p.how {
            How::Replay => PartStart::Fresh,
            How::Save(mode) => {
                let save = rerun.plan.restore.get(&p.part).cloned().ok_or_else(|| {
                    ForkError::Internal(anyhow::anyhow!("there is no save of the part {}", p.part))
                })?;
                PartStart::Save(FromSave { save, mode })
            }
        };
        out.insert(p.part.clone(), start);
    }
    Ok(out)
}

/// A replay's failure as a fork error naming the part.
fn replay_error(part: &str, e: ReplayError) -> ForkError {
    let part = part.to_string();
    match e {
        ReplayError::Diverged { seq } => ForkError::ReplayDiverged { part, seq },
        ReplayError::Fingerprint => ForkError::FingerprintMismatch { part },
        ReplayError::Failed(e) => ForkError::Internal(e.into()),
    }
}

/// A failed world start: a part's fingerprint refusal names that part.
fn start_error(e: anyhow::Error) -> ForkError {
    let refused = e
        .chain()
        .filter_map(|c| c.downcast_ref::<WorldError>())
        .any(|w| w.code == Some(replay::FINGERPRINT_MISMATCH));
    match e.downcast_ref::<StartingPart>() {
        Some(StartingPart(part)) if refused => {
            ForkError::FingerprintMismatch { part: part.clone() }
        }
        _ => ForkError::Internal(e),
    }
}

/// Starts the new branch's world: each part from its save, or fresh and then replayed from the
/// branch's recorded `events` up to the fork point. On a failure every instance is stopped.
pub async fn start_world(
    state: &AppState,
    events: &[Event],
    new_id: &str,
    rerun: &Rerun,
) -> Result<World, ForkError> {
    let from = WorldFrom::Saves(start_saves(rerun)?);
    let world = World::start(
        state.hub.clone(),
        &rerun.bench,
        new_id,
        rerun.run.seed,
        from,
    )
    .await
    .map_err(start_error)?;
    for p in rerun.replayed() {
        let inputs = replay::inputs(events, &p.part, &p.tools, rerun.plan.start_seq);
        if let Err(e) = replay::replay(&world, &p.part, &inputs, rerun.clock_ms).await {
            world.stop().await;
            return Err(replay_error(&p.part, e));
        }
    }
    Ok(world)
}

/// One part of a restore plan, as the rerun panel shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PartPlan {
    pub part: String,
    pub how: RestoredHow,
    /// For a replay: a rough estimate of how long it takes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimate_seconds: Option<f64>,
}

/// Each part of a planned rerun and how it is restored, with an estimate for replays.
pub fn part_plans(rerun: &Rerun, events: &[Event]) -> Vec<PartPlan> {
    let start = rerun.plan.start_seq;
    rerun
        .parts
        .iter()
        .map(|p| PartPlan {
            part: p.part.clone(),
            how: p.how.label(),
            estimate_seconds: (p.how == How::Replay)
                .then(|| replay::estimate_seconds(events, &p.part, &p.tools, start)),
        })
        .collect()
}
