use super::*;
use crate::suite::fixtures::*;

const POKED: Chk = ("poked", true, false);
const MISSED: Chk = ("poked", false, false);
const SAFE: Chk = ("not_overpoked", true, true);
const UNSAFE: Chk = ("not_overpoked", false, true);

fn two_by_two() -> SuiteReport {
    let s = suite("s", &["t1", "t2"], 2);
    let runs = filled(
        &s,
        vec![
            ran(&[POKED, SAFE], 2.0, 100, Some(0.01)),
            ran(&[POKED, UNSAFE], 4.0, 300, None),
            ran(&[MISSED, SAFE], 6.0, 200, Some(0.03)),
            infra_error(),
        ],
    );
    report(s, runs)
}

#[test]
fn a_two_by_two_suite_aggregates_its_checks_per_task() {
    let r = two_by_two();
    let rates: Vec<(&str, &str, bool, i64, i64)> = r
        .checks
        .iter()
        .map(|c| {
            (
                c.task_id.as_str(),
                c.name.as_str(),
                c.safety,
                c.passed,
                c.runs,
            )
        })
        .collect();
    assert_eq!(
        rates,
        vec![
            ("t1", "poked", false, 2, 2),
            ("t1", "not_overpoked", true, 1, 2),
            ("t2", "poked", false, 0, 1),
            ("t2", "not_overpoked", true, 1, 1),
        ]
    );
    assert_eq!(r.checks[1].pass_rate, 0.5);
    let statuses: Vec<&str> = r.runs.iter().map(|x| x.status.as_str()).collect();
    assert_eq!(statuses, vec!["passed", "failed", "failed", "infra_error"]);
}

#[test]
fn safety_violations_are_counted_apart_from_the_score() {
    let s = two_by_two().summary;
    assert_eq!(
        (s.runs, s.ended, s.scored, s.unscored, s.passed),
        (4, 4, 3, 1, 1)
    );
    assert_eq!(s.pass_rate, Some(1.0 / 3.0));
    // Goal checks only: two of three passed, though one of those runs was unsafe.
    assert_eq!(s.score, Some(2.0 / 3.0));
    assert_eq!((s.safety_violations, s.runs_with_violation), (1, 1));
}

#[test]
fn each_figure_is_spread_across_a_tasks_repeats() {
    let r = two_by_two();
    let t1 = &r.spread[0].figures;
    assert_eq!(r.spread[0].runs, 2);
    assert_eq!(
        (t1.measured_s.min, t1.measured_s.max, t1.measured_s.mean),
        (Some(2.0), Some(4.0), Some(3.0))
    );
    assert_eq!(
        (t1.tokens.mean, t1.peak_context.max),
        (Some(200.0), Some(150.0))
    );
    assert_eq!(
        (t1.cost_usd.n, t1.cost_usd.unknown, t1.cost_usd.mean),
        (1, 1, Some(0.01))
    );
    assert_eq!(r.spread[1].runs, 1, "the unscored run is left out");
    assert_eq!(r.summary.figures.checks_passed.mean, Some(2.0 / 3.0));
}

#[test]
fn a_suite_is_finished_when_every_run_ended_and_says_how_long_it_took() {
    let r = two_by_two();
    assert_eq!((r.status, r.finished_s), ("finished", Some(40.0)));
}

#[test]
fn a_suite_with_slots_left_is_running_or_stopped() {
    let s = suite("s", &["t1"], 2);
    let runs = filled(&s, vec![ran(&[POKED], 1.0, 1, None)]);
    let r = report(s.clone(), runs.clone());
    assert_eq!((r.status, r.finished_s), ("running", None));
    assert_eq!(r.runs[1].status, "pending");
    let stopped = Suite {
        error: Some("disk full".into()),
        ..s
    };
    assert_eq!(report(stopped, runs).status, "stopped");
}
