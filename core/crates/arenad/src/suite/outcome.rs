//! What one run of a suite came to: its status, its checks (with the safety ones marked), and the
//! figures its repeats are spread over (checks passed, time, tokens, cost, peak context).

use chrono::{DateTime, Utc};
use parts::proto::NamedCheck;
use serde::Serialize;

use super::safety;
use crate::{
    metrics::{self, Metrics},
    model::*,
    prices::Prices,
};

/// The one check of a run scored against the task's gold world instead of named checks.
pub const GOLD_CHECK: &str = "matches_gold";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunOutcome {
    pub status: BranchStatus,
    /// The branch's error, if any.
    pub error: Option<String>,
    /// The run's checks; empty until it was scored (or when it ended without being scored).
    pub checks: Vec<NamedCheck>,
    /// True once the run ended with a `check` event.
    pub scored: bool,
    /// Its checks that are not safety checks, and how many of them passed.
    pub checks_passed: i64,
    pub checks_total: i64,
    /// Its safety checks that failed.
    pub safety_violations: i64,
    /// Its tool calls the world refused as not allowed (`safety::is_denial`).
    pub denials: i64,
    /// Its agents' calls that raised a matter for a human (`safety::is_flag`).
    pub flags: i64,
    pub measured_s: f64,
    pub estimated_s: Option<f64>,
    pub tokens: i64,
    pub cost_usd: Option<f64>,
    /// The most context any of its model calls was sent with (prompt tokens).
    pub peak_context: Option<i64>,
}

/// The checks of a `check` event: the benchmark's named checks, or one `matches_gold` check for a
/// run compared with the task's gold world.
fn checks_of(check: &CheckPayload) -> Vec<NamedCheck> {
    match &check.checks {
        Some(named) => named.clone(),
        None => vec![NamedCheck {
            name: GOLD_CHECK.into(),
            passed: check.passed,
            detail: String::new(),
            safety: false,
        }],
    }
}

/// The run's last `check` event, read as a `CheckPayload`.
fn last_check(events: &[Event]) -> Option<CheckPayload> {
    let e = events.iter().rev().find(|e| e.kind == EventKind::Check)?;
    serde_json::from_value(e.payload.clone()).ok()
}

/// The largest peak context of the run's agents.
fn peak_context(m: &Metrics) -> Option<i64> {
    m.agents.iter().filter_map(|a| a.peak_prompt_tokens).max()
}

/// The outcome of a suite's run from its branch and events, at `now` while it runs.
pub fn outcome(
    branch: &Branch,
    events: &[Event],
    table: &Prices,
    now: DateTime<Utc>,
) -> RunOutcome {
    let m = metrics::metrics(branch, events, table, now);
    let check = last_check(events);
    let checks = check.as_ref().map(checks_of).unwrap_or_default();
    let (safety, goals): (Vec<&NamedCheck>, Vec<&NamedCheck>) =
        checks.iter().partition(|c| c.safety);
    let raised = safety::raised(events);
    RunOutcome {
        status: branch.status,
        error: branch.error.clone(),
        // The check is recorded just before the branch's end: a run is scored once it ended.
        scored: check.is_some() && branch.status != BranchStatus::Running,
        checks_passed: goals.iter().filter(|c| c.passed).count() as i64,
        checks_total: goals.len() as i64,
        safety_violations: safety.iter().filter(|c| !c.passed).count() as i64,
        denials: raised.denials,
        flags: raised.flags,
        checks,
        measured_s: m.measured_s,
        estimated_s: m.estimated_s,
        tokens: m.tokens,
        cost_usd: m.cost.usd,
        peak_context: peak_context(&m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;
    use serde_json::{json, Value};

    fn branch(status: BranchStatus) -> Branch {
        let b = testkit::branch(status);
        Branch {
            ended_at: Some(b.created_at + chrono::Duration::seconds(3)),
            ..b
        }
    }

    fn check_event(payload: Value) -> Event {
        testkit::event(9, None, EventKind::Check, payload)
    }

    #[test]
    fn safety_checks_are_counted_apart_from_the_score() {
        let checks = json!([{"name": "a", "passed": true}, {"name": "b", "passed": false},
                            {"name": "s", "passed": false, "safety": true}]);
        let ev = check_event(
            json!({"passed": false, "gold_db": null, "differing_rows": ["b", "s"],
                                    "checks": checks}),
        );
        let o = outcome(
            &branch(BranchStatus::Failed),
            &[ev],
            &Prices::default(),
            Utc::now(),
        );
        assert_eq!(
            (o.checks_passed, o.checks_total, o.safety_violations),
            (1, 2, 1)
        );
        assert!(o.scored && o.checks[2].safety);
        assert_eq!(o.measured_s, 3.0);
    }

    #[test]
    fn a_run_scored_against_gold_has_one_check_and_an_unscored_one_none() {
        let ev = check_event(json!({"passed": true, "gold_db": "g", "differing_rows": []}));
        let o = outcome(
            &branch(BranchStatus::Passed),
            &[ev],
            &Prices::default(),
            Utc::now(),
        );
        assert_eq!(
            (o.checks[0].name.as_str(), o.checks_passed),
            (GOLD_CHECK, 1)
        );
        let o = outcome(
            &branch(BranchStatus::InfraError),
            &[],
            &Prices::default(),
            Utc::now(),
        );
        assert!(!o.scored && o.checks.is_empty());
    }
}
