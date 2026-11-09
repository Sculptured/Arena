//! Experiments: one task of the benchmark in one scenario, run by 1 to 4 agent setups, each the
//! same number of times. Each setup is stored as a suite of that one task (its team, its runner,
//! repeats = the runs), tagged with the experiment, so suite reports, outcomes and comparisons
//! serve experiments as they are. arenad drives an experiment's runs one at a time, interleaved
//! for fairness: every setup's first run, then every setup's second, and so on.
//!
//! Suites made on their own, before experiments, read as one-setup experiments (`legacy`).

mod drive;
mod view;

use chrono::{DateTime, Utc};
use serde::Serialize;

pub use drive::{spawn, stop};
pub use view::{detail, legacy_detail, list, Detail, Head, Listed, SetupLine};

/// The most setups an experiment compares.
pub const MAX_SETUPS: usize = 4;
/// How many runs each setup gets when the request does not say.
pub const DEFAULT_RUNS: i32 = 5;
/// Why an experiment someone stopped ended early.
pub const STOPPED: &str = "stopped";

/// An experiment's definition and whether arenad still drives it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Experiment {
    pub id: String,
    pub name: String,
    pub task_id: String,
    pub benchmark: String,
    /// How many runs each setup gets.
    pub runs: i32,
    pub created_at: DateTime<Utc>,
    /// When arenad stopped driving it (every run done, or stopped on `error`).
    pub ended_at: Option<DateTime<Utc>>,
    /// Why it stopped before its last run (`stopped` when someone stopped it).
    pub error: Option<String>,
}

impl Experiment {
    /// `running` until it ended, then `stopped` when it ended early, else `finished`.
    pub fn status(&self) -> &'static str {
        match (self.ended_at, &self.error) {
            (None, _) => "running",
            (Some(_), Some(_)) => "stopped",
            (Some(_), None) => "finished",
        }
    }
}

/// The setups' names: each one asked for, trimmed, or `Setup A`, `Setup B`, … by its place.
/// An error when two are the same.
pub fn setup_names(asked: &[Option<&str>]) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = vec![];
    for (letter, name) in ('A'..).zip(asked) {
        let name = name
            .and_then(crate::model::trimmed)
            .unwrap_or_else(|| format!("Setup {letter}"));
        if names.contains(&name) {
            return Err(format!("two setups are called {name:?}"));
        }
        names.push(name);
    }
    Ok(names)
}

/// The experiment's name: the one asked for, trimmed, else its setups' names joined by ` vs `.
pub fn name_of(asked: Option<&str>, setups: &[String]) -> String {
    asked
        .and_then(crate::model::trimmed)
        .unwrap_or_else(|| setups.join(" vs "))
}

/// The order an experiment's runs are made in, as `(setup, run)` (both from 0): round by round,
/// each round every setup's next run. A setup with fewer runs drops out of later rounds.
pub fn run_order(runs: &[i32]) -> Vec<(usize, i32)> {
    let rounds = runs.iter().copied().max().unwrap_or(0);
    let round = |r| {
        (0..runs.len())
            .filter(move |&s| r < runs[s])
            .map(move |s| (s, r))
    };
    (0..rounds).flat_map(round).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setups_are_named_by_letter_unless_named_and_names_are_unique() {
        let names = setup_names(&[None, Some("  Rules "), Some(""), None]).unwrap();
        assert_eq!(names, ["Setup A", "Rules", "Setup C", "Setup D"]);
        let e = setup_names(&[Some("X"), Some(" X")]).unwrap_err();
        assert!(e.contains("\"X\""), "{e}");
        let e = setup_names(&[Some("Setup B"), None]).unwrap_err();
        assert!(e.contains("Setup B"), "{e}");
        assert_eq!(name_of(None, &names[..2]), "Setup A vs Rules");
        assert_eq!(name_of(Some(" Mine "), &names), "Mine");
    }

    #[test]
    fn runs_interleave_round_by_round() {
        assert_eq!(
            run_order(&[2, 2, 2]),
            [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (2, 1)]
        );
        assert_eq!(run_order(&[1, 2]), [(0, 0), (1, 0), (1, 1)]);
        assert!(run_order(&[]).is_empty());
    }
}
