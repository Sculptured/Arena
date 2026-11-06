//! A suite's report: each task × repeat with its run, the pass rate of every check per task,
//! safety violations counted apart from the score, the spread of each figure across repeats, and
//! the time the suite took to finish.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{RunOutcome, Slot, Suite};
use crate::model::BranchStatus;

/// One slot and what its run came to, as the report is built from.
#[derive(Debug, Clone)]
pub struct SlotRun {
    pub slot: Slot,
    /// The run's outcome; None until it has a branch, or when its branch is gone.
    pub outcome: Option<RunOutcome>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// One task × repeat in the report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunRow {
    pub task_id: String,
    pub repeat: i32,
    pub run_id: Option<String>,
    pub branch_id: Option<String>,
    /// `pending` (not claimed), `starting`, `error` (it could not start), `missing` (its run was
    /// deleted), or its branch's status.
    pub status: String,
    pub error: Option<String>,
    pub outcome: Option<RunOutcome>,
}

/// How often one check of one task passed over the suite's scored runs of that task.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckRate {
    pub task_id: String,
    pub name: String,
    pub safety: bool,
    pub passed: i64,
    pub runs: i64,
    pub pass_rate: f64,
}

/// A figure over a set of runs: how many had it (`n`) and how many did not (`unknown`), and its
/// smallest, largest and mean value among those that had it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Spread {
    pub n: usize,
    pub unknown: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
}

impl Spread {
    pub fn of(values: impl IntoIterator<Item = Option<f64>>) -> Spread {
        let mut s = Spread::default();
        let mut sum = 0.0;
        for v in values {
            let Some(v) = v else {
                s.unknown += 1;
                continue;
            };
            s.n += 1;
            sum += v;
            s.min = Some(s.min.map_or(v, |m| m.min(v)));
            s.max = Some(s.max.map_or(v, |m| m.max(v)));
        }
        s.mean = (s.n > 0).then(|| sum / s.n as f64);
        s
    }
}

/// The spread of each figure over a set of scored runs.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Figures {
    pub checks_passed: Spread,
    pub measured_s: Spread,
    pub estimated_s: Spread,
    pub tokens: Spread,
    pub cost_usd: Spread,
    pub peak_context: Spread,
}

impl Figures {
    fn of(runs: &[&RunOutcome]) -> Figures {
        let spread = |f: fn(&RunOutcome) -> Option<f64>| Spread::of(runs.iter().map(|o| f(o)));
        Figures {
            checks_passed: spread(|o| Some(o.checks_passed as f64)),
            measured_s: spread(|o| Some(o.measured_s)),
            estimated_s: spread(|o| o.estimated_s),
            tokens: spread(|o| Some(o.tokens as f64)),
            cost_usd: spread(|o| o.cost_usd),
            peak_context: spread(|o| o.peak_context.map(|p| p as f64)),
        }
    }
}

/// One task's figures across its repeats.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TaskSpread {
    pub task_id: String,
    pub runs: usize,
    #[serde(flatten)]
    pub figures: Figures,
}

/// The suite as a whole. Only scored runs (ended with a check) count in the rates and spreads;
/// runs that ended without one (an infrastructure error, an interruption) are `unscored`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    pub runs: usize,
    pub ended: usize,
    pub scored: usize,
    pub unscored: usize,
    pub passed: usize,
    /// Scored runs that passed every check, safety checks included.
    pub pass_rate: Option<f64>,
    /// The share of goal checks (not safety checks) passed over every scored run.
    pub score: Option<f64>,
    /// Failed safety checks, over every scored run, and the runs with at least one.
    pub safety_violations: i64,
    pub runs_with_violation: usize,
    #[serde(flatten)]
    pub figures: Figures,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuiteReport {
    #[serde(flatten)]
    pub suite: Suite,
    /// `running`, `finished`, or `stopped` (a built-in suite that ended before its last run).
    pub status: &'static str,
    /// Seconds from the suite's creation to its last run's end, once it is finished.
    pub finished_s: Option<f64>,
    pub runs: Vec<RunRow>,
    pub checks: Vec<CheckRate>,
    pub summary: Summary,
    pub spread: Vec<TaskSpread>,
}

fn row_status(s: &SlotRun) -> String {
    match (&s.outcome, &s.slot) {
        (Some(o), _) => o.status.as_str().into(),
        (None, slot) if slot.error.is_some() => "error".into(),
        (None, slot) if slot.branch_id.is_some() => "missing".into(),
        (None, slot) if slot.claimed => "starting".into(),
        (None, _) => "pending".into(),
    }
}

fn row(s: &SlotRun) -> RunRow {
    RunRow {
        task_id: s.slot.task_id.clone(),
        repeat: s.slot.repeat,
        run_id: s.slot.run_id.clone(),
        branch_id: s.slot.branch_id.clone(),
        status: row_status(s),
        error: s.slot.error.clone(),
        outcome: s.outcome.clone(),
    }
}

fn new_rate(task: &str, name: &str, safety: bool) -> CheckRate {
    CheckRate {
        task_id: task.into(),
        name: name.into(),
        safety,
        passed: 0,
        runs: 0,
        pass_rate: 0.0,
    }
}

/// The scored outcomes among `runs`.
fn scored<'a>(runs: impl Iterator<Item = &'a SlotRun>) -> impl Iterator<Item = &'a RunOutcome> {
    runs.filter_map(|r| r.outcome.as_ref()).filter(|o| o.scored)
}

/// The scored outcomes of `task`.
fn scored_of<'a>(runs: &'a [SlotRun], task: &'a str) -> Vec<&'a RunOutcome> {
    scored(runs.iter().filter(move |r| r.slot.task_id == task)).collect()
}

/// One task's checks with their pass rates, in the order the runs list them.
fn task_rates(task: &str, done: &[&RunOutcome]) -> Vec<CheckRate> {
    let mut out: Vec<CheckRate> = vec![];
    for c in done.iter().flat_map(|o| &o.checks) {
        let i = match out.iter().position(|r| r.name == c.name) {
            Some(i) => i,
            None => {
                out.push(new_rate(task, &c.name, c.safety));
                out.len() - 1
            }
        };
        let rate = &mut out[i];
        rate.runs += 1;
        rate.passed += c.passed as i64;
        rate.pass_rate = rate.passed as f64 / rate.runs as f64;
    }
    out
}

/// Every task's checks with their pass rates, in the suite's task order.
fn check_rates(suite: &Suite, runs: &[SlotRun]) -> Vec<CheckRate> {
    let rates = |t: &String| task_rates(t, &scored_of(runs, t));
    suite.tasks.iter().flat_map(rates).collect()
}

fn ratio(a: i64, b: i64) -> Option<f64> {
    (b > 0).then(|| a as f64 / b as f64)
}

fn summary(runs: &[SlotRun]) -> Summary {
    let done: Vec<&RunOutcome> = scored(runs.iter()).collect();
    let ended = runs.iter().filter_map(|r| r.outcome.as_ref());
    let ended = ended.filter(|o| o.status != BranchStatus::Running).count();
    let passed = done
        .iter()
        .filter(|o| o.status == BranchStatus::Passed)
        .count();
    let goals: i64 = done.iter().map(|o| o.checks_total).sum();
    let goals_passed: i64 = done.iter().map(|o| o.checks_passed).sum();
    Summary {
        runs: runs.len(),
        ended,
        scored: done.len(),
        unscored: ended.saturating_sub(done.len()),
        passed,
        pass_rate: ratio(passed as i64, done.len() as i64),
        score: ratio(goals_passed, goals),
        safety_violations: done.iter().map(|o| o.safety_violations).sum(),
        runs_with_violation: done.iter().filter(|o| o.safety_violations > 0).count(),
        figures: Figures::of(&done),
    }
}

fn task_spreads(suite: &Suite, runs: &[SlotRun]) -> Vec<TaskSpread> {
    let spread = |task: &String| {
        let done = scored_of(runs, task);
        TaskSpread {
            task_id: task.clone(),
            runs: done.len(),
            figures: Figures::of(&done),
        }
    };
    suite.tasks.iter().map(spread).collect()
}

/// `running` while a slot waits or a run runs; `stopped` when a built-in suite gave up with
/// slots left; else `finished`. Takes each slot with its branch's status, if it has a branch.
pub fn status(suite: &Suite, slots: &[(&Slot, Option<BranchStatus>)]) -> &'static str {
    let waiting = slots.iter().any(|(s, _)| !s.claimed);
    let busy = |(s, b): &(&Slot, Option<BranchStatus>)| match b {
        Some(b) => *b == BranchStatus::Running,
        None => s.claimed && s.branch_id.is_none() && s.error.is_none(),
    };
    match (waiting, suite.error.is_some()) {
        (true, true) => "stopped",
        (true, false) => "running",
        _ if slots.iter().any(busy) => "running",
        _ => "finished",
    }
}

/// Seconds from the suite's creation to its last run's end.
fn finished_s(suite: &Suite, runs: &[SlotRun]) -> Option<f64> {
    let last = runs
        .iter()
        .filter_map(|r| r.ended_at)
        .max()
        .or(suite.ended_at)?;
    Some((last - suite.created_at).num_milliseconds() as f64 / 1000.0)
}

/// The report of `suite` from its slots and their runs.
pub fn report(suite: Suite, runs: Vec<SlotRun>) -> SuiteReport {
    let slots: Vec<_> = runs
        .iter()
        .map(|r| (&r.slot, r.outcome.as_ref().map(|o| o.status)))
        .collect();
    let status = status(&suite, &slots);
    let by_order: BTreeMap<i32, &SlotRun> = runs.iter().map(|r| (r.slot.ord, r)).collect();
    SuiteReport {
        status,
        finished_s: (status == "finished")
            .then(|| finished_s(&suite, &runs))
            .flatten(),
        runs: by_order.values().map(|r| row(r)).collect(),
        checks: check_rates(&suite, &runs),
        summary: summary(&runs),
        spread: task_spreads(&suite, &runs),
        suite,
    }
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
