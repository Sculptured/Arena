//! Test suites: one team run over a set of the benchmark's tasks, each repeated N times on the
//! same seeded world and scored by the same checks, then aggregated (pass rate per check, safety
//! violations apart from the score, spread across repeats) and compared with another suite.
//!
//! A suite is stored as its definition and one slot per task × repeat; a slot gets the run that
//! filled it. Everything else (status, aggregates) is worked out from the runs' records on read.

mod compare;
mod drive;
#[cfg(test)]
mod fixtures;
mod load;
mod outcome;
mod report;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Runner, Team};
use crate::scenario::Scenario;

pub use compare::{compare, Comparison, Verdict};
pub use drive::{claim_external, spawn_builtin, ExternalClaim};
pub use load::{listed, load, Listed};
pub use outcome::{outcome, RunOutcome, GOLD_CHECK};
pub use report::{report, CheckRate, Spread, SuiteReport};

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
}

/// One task × repeat of a suite and the run that filled it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    /// Its place in the order the runs are made (task by task, repeat by repeat).
    pub ord: i32,
    pub task_id: String,
    /// 1-based.
    pub repeat: i32,
    /// True once a runner took it (its run may still be starting).
    pub claimed: bool,
    pub run_id: Option<String>,
    pub branch_id: Option<String>,
    /// Why its run could not start.
    pub error: Option<String>,
}

/// The slots of `tasks` × `repeats`, in run order: every repeat of the first task, then the next.
pub fn slots_of(tasks: &[String], repeats: i32) -> Vec<Slot> {
    let pairs = tasks
        .iter()
        .flat_map(|t| (1..=repeats).map(move |r| (t, r)));
    pairs
        .enumerate()
        .map(|(i, (task_id, repeat))| Slot {
            ord: i as i32,
            task_id: task_id.clone(),
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
    fn slots_go_task_by_task_then_repeat_by_repeat() {
        let s = slots_of(&["a".into(), "b".into()], 2);
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
