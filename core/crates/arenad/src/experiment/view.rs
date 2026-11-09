//! Experiments as the API shows them: a line per experiment in the list and an experiment's
//! detail (its setups' suite reports). A suite made on its own reads as a one-setup experiment
//! with the suite's id, marked `legacy`.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::Experiment;
use crate::{
    model::Runner,
    scenario::Scenario,
    store::Store,
    suite::{self, Suite, SuiteReport},
};

/// What the list and the detail both say of an experiment.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Head {
    pub id: String,
    pub name: String,
    pub tasks: Vec<String>,
    pub benchmark: String,
    /// How many runs each setup gets (per task, for a legacy suite).
    pub runs: i32,
    pub created_at: DateTime<Utc>,
    /// `running`, `finished` or `stopped`.
    pub status: &'static str,
    /// True for a suite made on its own, shown as a one-setup experiment.
    pub legacy: bool,
}

impl Head {
    fn of(e: &Experiment) -> Head {
        Head {
            id: e.id.clone(),
            name: e.name.clone(),
            tasks: vec![e.task_id.clone()],
            benchmark: e.benchmark.clone(),
            runs: e.runs,
            created_at: e.created_at,
            status: e.status(),
            legacy: false,
        }
    }

    fn of_legacy(s: &Suite, status: &'static str) -> Head {
        Head {
            id: s.id.clone(),
            name: s.name.clone(),
            tasks: s.tasks.clone(),
            benchmark: s.benchmark.clone(),
            runs: s.repeats,
            created_at: s.created_at,
            status,
            legacy: true,
        }
    }
}

/// One setup in the list: its suite, name, team and runner, and how many of its runs ended and
/// passed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SetupLine {
    pub suite_id: String,
    pub name: String,
    pub team_name: String,
    pub runner: Runner,
    pub runs: usize,
    pub ended: usize,
    pub passed: usize,
}

impl SetupLine {
    fn of(l: suite::Listed) -> SetupLine {
        SetupLine {
            suite_id: l.suite.id,
            name: l.suite.name,
            team_name: l.suite.team.name,
            runner: l.suite.runner,
            runs: l.runs,
            ended: l.ended,
            passed: l.passed,
        }
    }
}

/// An experiment in the list.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub head: Head,
    pub setups: Vec<SetupLine>,
}

/// An experiment with its scenario and each setup's suite report, in setup order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub head: Head,
    pub ended_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// The expanded scenario every run starts in.
    pub scenario: Scenario,
    pub setups: Vec<SuiteReport>,
}

/// Every experiment and every suite made on its own, newest first.
pub async fn list(store: &Store) -> Result<Vec<Listed>> {
    let mut of_experiment: HashMap<String, Vec<Suite>> = HashMap::new();
    let mut out = vec![];
    for s in store.suites().await? {
        match &s.experiment_id {
            Some(e) => of_experiment.entry(e.clone()).or_default().push(s),
            None => out.push(legacy_line(store, s).await?),
        }
    }
    for e in store.experiments().await? {
        let mut suites = of_experiment.remove(&e.id).unwrap_or_default();
        suites.sort_by_key(|s| s.setup_ord);
        let mut setups = vec![];
        for s in suites {
            setups.push(SetupLine::of(suite::listed(store, s).await?));
        }
        let head = Head::of(&e);
        out.push(Listed { head, setups });
    }
    out.sort_by_key(|l| std::cmp::Reverse(l.head.created_at));
    Ok(out)
}

async fn legacy_line(store: &Store, s: Suite) -> Result<Listed> {
    let line = suite::listed(store, s).await?;
    let head = Head::of_legacy(&line.suite, line.status);
    Ok(Listed {
        head,
        setups: vec![SetupLine::of(line)],
    })
}

/// The experiment's detail, with its suites' reports.
pub async fn detail(store: &Store, e: &Experiment) -> Result<Detail> {
    let mut setups = vec![];
    for s in store.experiment_suites(&e.id).await? {
        setups.push(suite::load(store, s).await?);
    }
    let scenario = setups.first().map(|r| r.suite.scenario.clone());
    Ok(Detail {
        head: Head::of(e),
        ended_at: e.ended_at,
        error: e.error.clone(),
        scenario: scenario.unwrap_or_default(),
        setups,
    })
}

/// A suite made on its own as a one-setup experiment's detail.
pub async fn legacy_detail(store: &Store, s: Suite) -> Result<Detail> {
    let report = suite::load(store, s).await?;
    let s = &report.suite;
    Ok(Detail {
        head: Head::of_legacy(s, report.status),
        ended_at: s.ended_at,
        error: s.error.clone(),
        scenario: s.scenario.clone(),
        setups: vec![report],
    })
}
