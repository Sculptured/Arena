//! A suite's report and its line in the list, read from the store.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{
    outcome,
    report::{self, SlotRun},
    RunOutcome, Suite, SuiteReport,
};
use crate::{model::BranchStatus, prices, store::Store};

/// The outcome of the run on `branch` and when it ended; None when the branch is gone.
async fn ran(
    store: &Store,
    branch: &str,
    now: DateTime<Utc>,
) -> Result<Option<(RunOutcome, Option<DateTime<Utc>>)>> {
    let Some(b) = store.find_branch(branch).await? else {
        return Ok(None);
    };
    let events = store.events(branch).await?;
    Ok(Some((
        outcome(&b, &events, prices::table(), now),
        b.ended_at,
    )))
}

/// The suite's report, from its slots and their runs' records.
pub async fn load(store: &Store, suite: Suite) -> Result<SuiteReport> {
    let now = Utc::now();
    let mut runs = vec![];
    for slot in store.slots(&suite.id).await? {
        let got = match &slot.branch_id {
            Some(b) => ran(store, b, now).await?,
            None => None,
        };
        let (outcome, ended_at) = got.map_or((None, None), |(o, e)| (Some(o), e));
        runs.push(SlotRun {
            slot,
            outcome,
            ended_at,
        });
    }
    Ok(report::report(suite, runs))
}

/// A suite in the list: its definition, status, and how many of its runs ended and passed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    #[serde(flatten)]
    pub suite: Suite,
    pub status: &'static str,
    pub runs: usize,
    pub ended: usize,
    pub passed: usize,
}

/// The suite's line in the list (branch statuses only, no events read).
pub async fn listed(store: &Store, suite: Suite) -> Result<Listed> {
    let slots = store.slots(&suite.id).await?;
    let mut statuses = vec![];
    for s in &slots {
        let b = match &s.branch_id {
            Some(id) => store.find_branch(id).await?.map(|b| b.status),
            None => None,
        };
        statuses.push((s, b));
    }
    let ended = statuses
        .iter()
        .filter(|(_, b)| b.is_some_and(|b| b != BranchStatus::Running));
    let passed = statuses
        .iter()
        .filter(|(_, b)| *b == Some(BranchStatus::Passed));
    Ok(Listed {
        status: report::status(&suite, &statuses),
        runs: slots.len(),
        ended: ended.count(),
        passed: passed.count(),
        suite,
    })
}
