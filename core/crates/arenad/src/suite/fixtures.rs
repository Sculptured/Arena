//! Suites and outcomes for the suite module's unit tests.

use chrono::{DateTime, TimeZone, Utc};
use parts::proto::NamedCheck;

use super::{report::SlotRun, slots_of, RunOutcome, Suite};
use crate::model::{BranchStatus, Runner, Team};

pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap()
}

pub fn suite(id: &str, tasks: &[&str], repeats: i32) -> Suite {
    Suite {
        id: id.into(),
        name: id.into(),
        team: Team {
            id: "t".into(),
            name: "T".into(),
            agents: vec![],
        },
        tasks: tasks.iter().map(|t| t.to_string()).collect(),
        repeats,
        runner: Runner::Chain,
        benchmark: "b".into(),
        created_at: t0(),
        ended_at: None,
        error: None,
        scenario: Default::default(),
        variants: vec![],
    }
}

/// `(name, passed, safety)`.
pub type Chk = (&'static str, bool, bool);

/// A scored run's outcome with `checks`, `measured_s`, `tokens` and `cost_usd`.
pub fn ran(checks: &[Chk], measured_s: f64, tokens: i64, cost_usd: Option<f64>) -> RunOutcome {
    let checks: Vec<NamedCheck> = checks
        .iter()
        .map(|&(name, passed, safety)| NamedCheck {
            name: name.into(),
            passed,
            detail: String::new(),
            safety,
        })
        .collect();
    let goals = checks.iter().filter(|c| !c.safety);
    let all_passed = checks.iter().all(|c| c.passed);
    RunOutcome {
        status: if all_passed {
            BranchStatus::Passed
        } else {
            BranchStatus::Failed
        },
        error: None,
        scored: true,
        checks_passed: goals.clone().filter(|c| c.passed).count() as i64,
        checks_total: goals.count() as i64,
        safety_violations: checks.iter().filter(|c| c.safety && !c.passed).count() as i64,
        checks,
        measured_s,
        estimated_s: Some(measured_s / 2.0),
        tokens,
        cost_usd,
        peak_context: Some(tokens / 2),
    }
}

/// A run that ended without being scored.
pub fn infra_error() -> RunOutcome {
    RunOutcome {
        status: BranchStatus::InfraError,
        scored: false,
        checks: vec![],
        checks_passed: 0,
        checks_total: 0,
        safety_violations: 0,
        error: Some("boom".into()),
        ..ran(&[], 1.0, 0, None)
    }
}

/// The slots of `suite`, filled in order with `outcomes` (each ending 10 s after the last).
pub fn filled(suite: &Suite, outcomes: Vec<RunOutcome>) -> Vec<SlotRun> {
    let mut slots = slots_of(&suite.tasks, 1, suite.repeats).into_iter();
    let mut out = vec![];
    for (i, o) in outcomes.into_iter().enumerate() {
        let mut slot = slots.next().expect("a slot per outcome");
        slot.claimed = true;
        slot.branch_id = Some(format!("b{i}"));
        let ended_at = Some(t0() + chrono::Duration::seconds(10 * (i as i64 + 1)));
        out.push(SlotRun {
            slot,
            outcome: Some(o),
            ended_at,
        });
    }
    out.extend(slots.map(|slot| SlotRun {
        slot,
        outcome: None,
        ended_at: None,
    }));
    out
}
