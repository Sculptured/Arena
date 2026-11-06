//! Two suites side by side (two deployments, or one before and after a change): each check's
//! pass rate and its change, safety, time, tokens and cost, each flagged improved, regressed or
//! the same. The regression test before production.

use serde::Serialize;

use super::{report::Spread, CheckRate, SuiteReport};

/// How `b` did against `a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Improved,
    Regressed,
    Same,
    /// Only one side has the figure (a check one suite never ran, a cost one never knew).
    Incomparable,
}

/// Time, tokens and cost within this share of `a`'s value are the same: runs vary a little.
pub const TOLERANCE: f64 = 0.10;

/// Pass rates closer than this are equal.
const EPSILON: f64 = 1e-9;

/// One figure of both suites and its change (`b - a`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Delta {
    pub a: Option<f64>,
    pub b: Option<f64>,
    pub change: Option<f64>,
    pub verdict: Verdict,
}

impl Delta {
    /// A rate where higher is better: any change counts.
    fn higher_better(a: Option<f64>, b: Option<f64>) -> Delta {
        Delta::new(a, b, |a, b| verdict_of(b - a, EPSILON))
    }

    /// A rate where lower is better: any change counts.
    fn lower_better(a: Option<f64>, b: Option<f64>) -> Delta {
        Delta::new(a, b, |a, b| verdict_of(a - b, EPSILON))
    }

    /// A cost (time, tokens, dollars): lower is better, beyond `TOLERANCE` of `a`.
    fn cost(a: Option<f64>, b: Option<f64>) -> Delta {
        Delta::new(a, b, |a, b| {
            verdict_of(a - b, (a.abs() * TOLERANCE).max(EPSILON))
        })
    }

    fn new(a: Option<f64>, b: Option<f64>, judge: impl Fn(f64, f64) -> Verdict) -> Delta {
        let both = a.zip(b);
        Delta {
            a,
            b,
            change: both.map(|(a, b)| b - a),
            verdict: both.map_or(Verdict::Incomparable, |(a, b)| judge(a, b)),
        }
    }
}

/// Improved when `gain` is above `margin`, regressed when below `-margin`.
fn verdict_of(gain: f64, margin: f64) -> Verdict {
    if gain > margin {
        Verdict::Improved
    } else if gain < -margin {
        Verdict::Regressed
    } else {
        Verdict::Same
    }
}

/// One check of one task in both suites.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckDelta {
    pub task_id: String,
    pub name: String,
    pub safety: bool,
    #[serde(flatten)]
    pub rate: Delta,
}

/// Which suite a side is.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Side {
    pub id: String,
    pub name: String,
    pub team: String,
    pub status: &'static str,
    pub scored: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comparison {
    pub a: Side,
    pub b: Side,
    /// Regressed when a check or safety got worse; improved when something got better and
    /// nothing worse; else the same. Time and cost are flagged on their own and don't count.
    pub verdict: Verdict,
    pub checks: Vec<CheckDelta>,
    pub pass_rate: Delta,
    pub score: Delta,
    /// Failed safety checks per scored run.
    pub safety: Delta,
    pub measured_s: Delta,
    pub estimated_s: Delta,
    pub tokens: Delta,
    pub cost_usd: Delta,
    pub peak_context: Delta,
}

fn side(r: &SuiteReport) -> Side {
    Side {
        id: r.suite.id.clone(),
        name: r.suite.name.clone(),
        team: r.suite.team.name.clone(),
        status: r.status,
        scored: r.summary.scored,
    }
}

/// Every task × check of either suite, `a`'s first, with both pass rates.
fn check_deltas(a: &[CheckRate], b: &[CheckRate]) -> Vec<CheckDelta> {
    let find = |list: &[CheckRate], c: &CheckRate| {
        let same = |x: &&CheckRate| x.task_id == c.task_id && x.name == c.name;
        list.iter().find(same).map(|x| x.pass_rate)
    };
    let only_b = b.iter().filter(|c| find(a, c).is_none());
    a.iter()
        .chain(only_b)
        .map(|c| CheckDelta {
            task_id: c.task_id.clone(),
            name: c.name.clone(),
            safety: c.safety,
            rate: Delta::higher_better(find(a, c), find(b, c)),
        })
        .collect()
}

fn safety_rate(r: &SuiteReport) -> Option<f64> {
    let s = &r.summary;
    (s.scored > 0).then(|| s.safety_violations as f64 / s.scored as f64)
}

fn overall(deltas: &[&Delta]) -> Verdict {
    let any = |v: Verdict| deltas.iter().any(|d| d.verdict == v);
    match (any(Verdict::Regressed), any(Verdict::Improved)) {
        (true, _) => Verdict::Regressed,
        (false, true) => Verdict::Improved,
        _ => Verdict::Same,
    }
}

/// `b` against `a`.
pub fn compare(a: &SuiteReport, b: &SuiteReport) -> Comparison {
    let (fa, fb) = (&a.summary.figures, &b.summary.figures);
    let cost = |f: fn(&super::report::Figures) -> &Spread| Delta::cost(f(fa).mean, f(fb).mean);
    let checks = check_deltas(&a.checks, &b.checks);
    let safety = Delta::lower_better(safety_rate(a), safety_rate(b));
    let pass_rate = Delta::higher_better(a.summary.pass_rate, b.summary.pass_rate);
    let mut judged: Vec<&Delta> = checks.iter().map(|c| &c.rate).collect();
    judged.extend([&safety, &pass_rate]);
    Comparison {
        a: side(a),
        b: side(b),
        verdict: overall(&judged),
        score: Delta::higher_better(a.summary.score, b.summary.score),
        measured_s: cost(|f| &f.measured_s),
        estimated_s: cost(|f| &f.estimated_s),
        tokens: cost(|f| &f.tokens),
        cost_usd: cost(|f| &f.cost_usd),
        peak_context: cost(|f| &f.peak_context),
        checks,
        safety,
        pass_rate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::{fixtures::*, report};

    const POKED: Chk = ("poked", true, false);
    const MISSED: Chk = ("poked", false, false);
    const SAFE: Chk = ("not_overpoked", true, true);
    const UNSAFE: Chk = ("not_overpoked", false, true);

    fn suite_of(id: &str, outcomes: Vec<crate::suite::RunOutcome>) -> SuiteReport {
        let s = suite(id, &["t1", "t2"], 1);
        let runs = filled(&s, outcomes);
        report(s, runs)
    }

    /// `a` misses t2's goal; `b` meets it but trips t1's safety check, faster, with more tokens.
    fn a_and_b() -> Comparison {
        let a = suite_of(
            "a",
            vec![
                ran(&[POKED, SAFE], 10.0, 100, Some(1.0)),
                ran(&[MISSED, SAFE], 10.0, 100, Some(1.0)),
            ],
        );
        let b = suite_of(
            "b",
            vec![
                ran(&[POKED, UNSAFE], 10.5, 300, Some(1.0)),
                ran(&[POKED, SAFE], 5.0, 100, None),
            ],
        );
        compare(&a, &b)
    }

    #[test]
    fn compare_flags_a_regression_and_an_improvement_per_check() {
        let c = a_and_b();
        let got: Vec<(&str, &str, Verdict)> = c
            .checks
            .iter()
            .map(|d| (d.task_id.as_str(), d.name.as_str(), d.rate.verdict))
            .collect();
        assert_eq!(
            got,
            vec![
                ("t1", "poked", Verdict::Same),
                ("t1", "not_overpoked", Verdict::Regressed),
                ("t2", "poked", Verdict::Improved),
                ("t2", "not_overpoked", Verdict::Same),
            ]
        );
        let s = &c.safety;
        assert_eq!(
            (s.a, s.b, s.verdict),
            (Some(0.0), Some(0.5), Verdict::Regressed)
        );
        assert_eq!(c.verdict, Verdict::Regressed);
    }

    #[test]
    fn compare_flags_time_tokens_and_cost_beyond_the_tolerance() {
        let c = a_and_b();
        // 10 -> 7.75 s mean is more than 10% faster; tokens doubled; b knows one cost, the same.
        assert_eq!(c.measured_s.verdict, Verdict::Improved);
        assert_eq!(
            (c.tokens.change, c.tokens.verdict),
            (Some(100.0), Verdict::Regressed)
        );
        assert_eq!(c.cost_usd.verdict, Verdict::Same);
    }

    #[test]
    fn identical_suites_are_the_same_and_a_check_on_one_side_is_incomparable() {
        let a = suite_of(
            "a",
            vec![
                ran(&[POKED], 10.0, 100, None),
                ran(&[POKED], 10.0, 100, None),
            ],
        );
        let c = compare(&a, &a);
        assert_eq!(
            (c.verdict, c.measured_s.verdict, c.cost_usd.verdict),
            (Verdict::Same, Verdict::Same, Verdict::Incomparable)
        );
        let b = suite_of(
            "b",
            vec![
                ran(&[POKED, SAFE], 10.0, 100, None),
                ran(&[POKED], 10.0, 100, None),
            ],
        );
        let c = compare(&a, &b);
        let extra = c.checks.iter().find(|d| d.name == "not_overpoked").unwrap();
        assert_eq!(
            (extra.rate.a, extra.rate.verdict),
            (None, Verdict::Incomparable)
        );
        assert_eq!(c.verdict, Verdict::Same);
    }
}
