//! Test suites: one team run over a set of the benchmark's tasks, each repeated N times on the
//! same seeded world and scored by the same checks, then aggregated (pass rate per check, safety
//! violations apart from the score, spread across repeats) and compared with another suite.
//!
//! A suite is stored as its definition and one slot per task × variant × repeat (a variant is one
//! of the suite's scenarios; a suite with one scenario or none has one variant); a slot gets the run
//! that filled it. Everything else (status, aggregates) is worked out from the runs' records on read.

mod compare;
mod drive;
#[cfg(test)]
pub(crate) mod fixtures;
mod load;
mod outcome;
mod report;
mod safety;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Runner, Team};
use crate::scenario::Scenario;

pub use compare::{compare, CheckDelta, Comparison, Delta, Verdict};
pub use drive::{claim_external, spawn_builtin, ExternalClaim};
pub use load::{listed, load, Listed};
pub use outcome::{outcome, RunOutcome, GOLD_CHECK};
pub use report::{report, CheckRate, RunRow, Safety, Spread, SuiteReport};
pub use safety::{is_denial, is_flag};

/// The most repeats a suite may ask for.
pub const MAX_REPEATS: i32 = 100;

/// A suite's definition: the team, the tasks, how many times each runs, and who drives the runs
/// (arenad's own runners one run after another, or an external system that claims each run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suite {
    pub id: String,
    pub name: String,
    pub team: Team,
    pub tasks: Vec<String>,
    pub repeats: i32,
    pub runner: Runner,
    pub benchmark: String,
    pub created_at: DateTime<Utc>,
    /// When arenad stopped driving a built-in suite (every run done, or stopped on `error`).
    pub ended_at: Option<DateTime<Utc>>,
    /// Why a built-in suite stopped before its last run.
    pub error: Option<String>,
    /// The expanded scenario every run of the suite starts in (empty: the tasks as they are).
    #[serde(default)]
    pub scenario: Scenario,
    /// The expanded scenarios each task × repeat runs in, one slot each (empty: just `scenario`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<Scenario>,
}

impl Suite {
    /// The scenario a slot's run starts in: its variant's, else the suite's one scenario.
    pub fn scenario_of(&self, slot: &Slot) -> &Scenario {
        let variant = usize::try_from(slot.variant).ok();
        let found = variant.and_then(|i| self.variants.get(i));
        found.unwrap_or(&self.scenario)
    }
}

/// How many variants each task × repeat runs in: one per scenario of `variants`, at least one.
pub fn variant_count(variants: &[Scenario]) -> i32 {
    i32::try_from(variants.len().max(1)).expect("a suite has few variants")
}

/// One task × repeat of a suite and the run that filled it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Its place in the order the runs are made (task by task, repeat by repeat).
    pub ord: i32,
    pub task_id: String,
    /// Which of the suite's `variants` the run starts in (0 when it has none).
    #[serde(default)]
    pub variant: i32,
    /// 1-based.
    pub repeat: i32,
    /// True once a runner took it (its run may still be starting).
    pub claimed: bool,
    pub run_id: Option<String>,
    pub branch_id: Option<String>,
    /// Why its run could not start.
    pub error: Option<String>,
}

/// The slots of `tasks` × `variants` × `repeats`, in run order: every repeat of the first task's
/// first variant, then its next variant, then the next task.
pub fn slots_of(tasks: &[String], variants: i32, repeats: i32) -> Vec<Slot> {
    let per_task = move |t| (0..variants).flat_map(move |v| (1..=repeats).map(move |r| (t, v, r)));
    let cells = tasks.iter().flat_map(per_task);
    cells
        .enumerate()
        .map(|(i, (task_id, variant, repeat))| Slot {
            ord: i as i32,
            task_id: task_id.clone(),
            variant,
            repeat,
            claimed: false,
            run_id: None,
            branch_id: None,
            error: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_go_task_by_task_then_variant_by_variant_then_repeat_by_repeat() {
        let s = slots_of(&["a".into()], 2, 2);
        let got: Vec<(i32, i32)> = s.iter().map(|x| (x.variant, x.repeat)).collect();
        assert_eq!(got, vec![(0, 1), (0, 2), (1, 1), (1, 2)]);
        let s = slots_of(&["a".into(), "b".into()], 1, 2);
        let got: Vec<(i32, &str, i32)> = s
            .iter()
            .map(|x| (x.ord, x.task_id.as_str(), x.repeat))
            .collect();
        assert_eq!(
            got,
            vec![(0, "a", 1), (1, "a", 2), (2, "b", 1), (3, "b", 2)]
        );
    }
}
