//! What a rerun restores and how: per part, from its save or by replay (the restore table, with
//! the restore asked for), the fork plan, the checks that an exact restore can be exact, the
//! fingerprint check for replays, and starting the new branch's world that way.

use std::collections::{BTreeMap, BTreeSet};

use parts::proto::{BenchDescribe, ExactKind, FromSave, Mode};
use serde::Serialize;

use super::{
    exact::{ancestry, approximate_on_the_way, world_changed},
    plan_fork_with, ForkError, ForkPlan,
};
use crate::{
    app::AppState,
    model::{Branch, Change, Event, RestoredHow},
    recorded::{clock_at, run_info, RunInfo},
    replay::{self, ReplayError},
    restore::{restore_plan, How},
    world::{self, PartStart, Plan, Saves, StartSaves, StartingPart, World, WorldError, WorldFrom},
};

/// A fork request: where, what changes, and the restore asked for.
#[derive(Debug, Clone)]
pub struct ForkRequest {
    pub at_seq: i64,
    pub changes: std::collections::HashMap<String, Change>,
    pub restore: Mode,
}

/// How one part of the new world is restored.
pub struct PartRestore {
    pub part: String,
    pub how: How,
    /// The tools the part lists (its replayable inputs).
    pub tools: Vec<String>,
    /// True when the part declares replay as its exact restore.
    pub declares_replay: bool,
}

/// Everything a rerun needs before it starts: the run, its benchmark and world as they are now,
/// the fork plan, each part's restore (in world order) and the clock to start at.
pub struct Rerun {
    pub run: RunInfo,
    pub bench: BenchDescribe,
    /// The world's plan, until the world is started from it.
    world: Option<Plan>,
    pub plan: ForkPlan,
    pub parts: Vec<PartRestore>,
    pub clock_ms: i64,
}

/// The parts restored by replay.
fn replayed(parts: &[PartRestore]) -> impl Iterator<Item = &PartRestore> {
    parts.iter().filter(|p| p.how == How::Replay)
}

/// The names of the parts restored by replay.
fn replayed_names(parts: &[PartRestore]) -> BTreeSet<String> {
    replayed(parts).map(|p| p.part.clone()).collect()
}

impl Rerun {
    fn replayed(&self) -> impl Iterator<Item = &PartRestore> {
        replayed(&self.parts)
    }

    /// Each part and how it is restored, by part name.
    pub fn hows(&self) -> BTreeMap<String, How> {
        self.parts.iter().map(|p| (p.part.clone(), p.how)).collect()
    }

    /// Refuses a rerun that can't be what it claims: in an exact one, a part whose world changed
    /// since the run or that was restored approximately on the way; in any, a replayed part
    /// whose integration's fingerprint changed.
    async fn check(&self, state: &AppState, branch: &Branch, mode: Mode) -> Result<(), ForkError> {
        let replayed = replayed_names(&self.parts);
        if mode == Mode::Exact {
            world_changed(&self.run.world, &self.bench.world, &self.hows())
                .map_err(ForkError::Refused)?;
            let forks = ancestry(&state.store, branch).await?;
            approximate_on_the_way(&forks, &replayed, self.plan.start_seq)
                .map_err(ForkError::Refused)?;
        }
        let world = self
            .world
            .as_ref()
            .expect("checked before the world starts");
        check_fingerprints(&self.run, world, &replayed)
    }
}

/// Each part's restore in `mode`, in world order; the first part that can't be is refused.
fn part_restores(world: &Plan, mode: Mode) -> Result<Vec<PartRestore>, ForkError> {
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
            declares_replay: p.describe.restore.exact == ExactKind::Replay,
        })
        .collect())
}

/// A part that declares replay and would start from a save it doesn't have at the start point
/// (it starts fresh in a run's first branch, so it has none before its first save) starts fresh
/// with the run's seed and is replayed up to the start point instead: exactly.
fn replay_unsaved(parts: &mut [PartRestore], restore: &Saves) {
    for p in parts.iter_mut() {
        if p.declares_replay && matches!(p.how, How::Save(_)) && !restore.contains_key(&p.part) {
            p.how = How::Replay;
        }
    }
}

/// Plans a rerun of `branch` (its `events`) for `req`.
pub async fn prepare(
    state: &AppState,
    branch: &Branch,
    events: &[Event],
    req: &ForkRequest,
) -> Result<Rerun, ForkError> {
    if !events.iter().any(|e| e.seq == req.at_seq) {
        return Err(ForkError::UnknownSeq);
    }
    let run = run_info(events);
    let bench = state.hub.bench_describe(&run.benchmark).await?;
    let world = world::plan(&state.hub, &bench).await?;
    let mut parts = part_restores(&world, req.restore)?;
    let skip = replayed_names(&parts);
    let plan = plan_fork_with(events, &branch.team, req.at_seq, &req.changes, &skip)?;
    replay_unsaved(&mut parts, &plan.restore);
    let rerun = Rerun {
        clock_ms: clock_at(events, plan.start_seq),
        run,
        bench,
        world: Some(world),
        plan,
        parts,
    };
    rerun.check(state, branch, req.restore).await?;
    Ok(rerun)
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

/// Starts the new branch's world: each part from its save and moved to the fork point's clock,
/// or fresh and then replayed from the branch's recorded `events` up to the fork point. On a
/// failure every instance is stopped.
pub async fn start_world(
    state: &AppState,
    events: &[Event],
    new_id: &str,
    rerun: &mut Rerun,
) -> Result<World, ForkError> {
    let from = WorldFrom::Saves(start_saves(rerun)?);
    let (hub, seed) = (state.hub.clone(), rerun.run.seed);
    let planned = match rerun.world.take() {
        Some(p) => p,
        None => world::plan(&hub, &rerun.bench).await?,
    };
    let world = World::start_planned(hub, &rerun.bench, planned, new_id, seed, from)
        .await
        .map_err(start_error)?;
    let ready = async {
        catch_up_saved(&world, rerun).await?;
        replay_parts(&world, events, rerun).await
    };
    if let Err(e) = ready.await {
        world.stop().await;
        return Err(e);
    }
    Ok(world)
}

/// Moves every part restored from a save to the start point's clock (a save holds the clock of
/// its step, and what the part reports on the way was recorded before the start point).
async fn catch_up_saved(world: &World, rerun: &Rerun) -> Result<(), ForkError> {
    let saved = rerun.parts.iter().filter(|p| matches!(p.how, How::Save(_)));
    for p in saved {
        let caught_up = world.advance_part(&p.part, rerun.clock_ms).await;
        caught_up.map_err(|e| ForkError::Internal(e.into()))?;
    }
    Ok(())
}

/// Replays every replayed part of a started world up to the fork point.
async fn replay_parts(world: &World, events: &[Event], rerun: &Rerun) -> Result<(), ForkError> {
    for p in rerun.replayed() {
        let inputs = replay::inputs(events, &p.part, &p.tools, rerun.plan.start_seq);
        replay::replay(world, &p.part, &inputs, rerun.clock_ms)
            .await
            .map_err(|e| replay_error(&p.part, e))?;
    }
    Ok(())
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
