//! The digest of two suites compared: who each deployment is, the figures B against A, and each
//! check whose pass rate moved, with example runs (branch ids) where it passed and failed, so the
//! reviewer can cite runs a person can open.

use std::collections::BTreeSet;

use serde::Serialize;

use super::text::one_line;
use crate::suite::{CheckDelta, Comparison, Delta, RunRow, SuiteReport, Verdict};

/// Example runs per check and outcome.
const EXAMPLES: usize = 3;
const MAX_MOVED: usize = 30;
const PROMPT_BYTES: usize = 400;

#[derive(Debug, Clone, Serialize)]
pub struct SuiteDigest {
    pub a: SuiteSide,
    pub b: SuiteSide,
    /// Arena's own verdict on B against A.
    pub verdict: Verdict,
    pub figures: Figures,
    pub checks_moved: Vec<MovedCheck>,
    pub checks_same: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SuiteSide {
    pub id: String,
    pub name: String,
    pub status: &'static str,
    pub runner: crate::model::Runner,
    pub team: String,
    pub agents: Vec<AgentBrief>,
    pub runs: usize,
    pub scored: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentBrief {
    pub id: String,
    pub kind: crate::model::AgentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Figures {
    pub pass_rate: Delta,
    pub score: Delta,
    pub safety_violations_per_run: Delta,
    pub seconds_per_run: Delta,
    pub tokens_per_run: Delta,
    pub cost_usd_per_run: Delta,
}

#[derive(Debug, Clone, Serialize)]
pub struct MovedCheck {
    #[serde(flatten)]
    pub delta: CheckDelta,
    pub failed_in_a: Vec<String>,
    pub passed_in_a: Vec<String>,
    pub failed_in_b: Vec<String>,
    pub passed_in_b: Vec<String>,
}

fn side(r: &SuiteReport) -> SuiteSide {
    let agent = |a: &crate::model::AgentSpec| AgentBrief {
        id: a.id.clone(),
        kind: a.kind,
        model: a.model.clone(),
        prompt: one_line(&a.prompt, PROMPT_BYTES),
    };
    SuiteSide {
        id: r.suite.id.clone(),
        name: r.suite.name.clone(),
        status: r.status,
        runner: r.suite.runner,
        team: r.suite.team.name.clone(),
        agents: r.suite.team.agents.iter().map(agent).collect(),
        runs: r.runs.len(),
        scored: r.summary.scored,
    }
}

/// The branches of `r`'s scored runs of `task` where `check` came out `passed`, at most
/// [`EXAMPLES`].
fn examples(r: &SuiteReport, task: &str, check: &str, passed: bool) -> Vec<String> {
    let with = |row: &&RunRow| {
        let o = row.outcome.as_ref().filter(|o| o.scored);
        let c = o.and_then(|o| o.checks.iter().find(|c| c.name == check));
        row.task_id == task && c.is_some_and(|c| c.passed == passed)
    };
    let ids = r
        .runs
        .iter()
        .filter(with)
        .filter_map(|row| row.branch_id.clone());
    ids.take(EXAMPLES).collect()
}

fn moved(a: &SuiteReport, b: &SuiteReport, d: &CheckDelta) -> MovedCheck {
    let ex = |r, passed| examples(r, &d.task_id, &d.name, passed);
    MovedCheck {
        delta: d.clone(),
        failed_in_a: ex(a, false),
        passed_in_a: ex(a, true),
        failed_in_b: ex(b, false),
        passed_in_b: ex(b, true),
    }
}

pub fn suite_digest(a: &SuiteReport, b: &SuiteReport, c: &Comparison) -> SuiteDigest {
    let moved_checks = c.checks.iter().filter(|d| d.rate.verdict != Verdict::Same);
    let checks_moved: Vec<MovedCheck> = moved_checks
        .take(MAX_MOVED)
        .map(|d| moved(a, b, d))
        .collect();
    let same = c
        .checks
        .iter()
        .filter(|d| d.rate.verdict == Verdict::Same)
        .count();
    SuiteDigest {
        a: side(a),
        b: side(b),
        verdict: c.verdict,
        figures: Figures {
            pass_rate: c.pass_rate.clone(),
            score: c.score.clone(),
            safety_violations_per_run: c.safety.clone(),
            seconds_per_run: c.measured_s.clone(),
            tokens_per_run: c.tokens.clone(),
            cost_usd_per_run: c.cost_usd.clone(),
        },
        checks_moved,
        checks_same: same,
    }
}

/// The branches of a suite's runs: what a suite review may cite for that side.
pub fn runs_of(r: &SuiteReport) -> BTreeSet<String> {
    r.runs
        .iter()
        .filter_map(|row| row.branch_id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{compare, fixtures::*, report};

    fn poked_reports() -> (SuiteReport, SuiteReport) {
        let sa = suite("a", &["t1"], 2);
        let sb = suite("b", &["t1"], 2);
        let a = report(
            sa.clone(),
            filled(
                &sa,
                vec![
                    ran(&[("poked", false, false)], 10.0, 100, None),
                    ran(&[("poked", true, false)], 10.0, 100, None),
                ],
            ),
        );
        let b = report(
            sb.clone(),
            filled(
                &sb,
                vec![
                    ran(&[("poked", true, false)], 10.0, 100, None),
                    ran(&[("poked", true, false)], 10.0, 100, None),
                ],
            ),
        );
        (a, b)
    }

    #[test]
    fn moved_checks_come_with_example_runs_of_each_side() {
        let (a, b) = poked_reports();
        let c = compare(&a, &b);
        let d = suite_digest(&a, &b, &c);
        assert_eq!(d.checks_moved.len(), 1);
        let m = &d.checks_moved[0];
        assert_eq!(m.delta.name, "poked");
        assert_eq!(
            (m.failed_in_a.as_slice(), m.passed_in_a.as_slice()),
            (&["b0".to_string()][..], &["b1".to_string()][..])
        );
        assert!(m.failed_in_b.is_empty());
        assert_eq!(m.passed_in_b.len(), 2);
        assert_eq!(d.verdict, Verdict::Improved);
        assert_eq!(runs_of(&a).len(), 2);
    }
}
