//! The digest of two recorded pharma runs (from the demo stack, 2026-11-08; world views left out
//! and long results trimmed for the fixture): a six-agent Claude Code haiku team that failed five
//! checks, and a single Claude Code Sonnet agent that failed one.

use super::*;

pub(crate) struct Recorded {
    pub branch: Branch,
    pub events: Vec<Event>,
}

pub(crate) fn recorded(text: &str) -> Recorded {
    let v: Value = serde_json::from_str(text).expect("a recorded branch is JSON");
    Recorded {
        branch: serde_json::from_value(v.clone()).expect("a recorded branch"),
        events: serde_json::from_value(v["events"].clone()).expect("its events"),
    }
}

pub(crate) fn team_run() -> Recorded {
    recorded(include_str!("testdata/team_haiku_failed.json"))
}

pub(crate) fn single_run() -> Recorded {
    recorded(include_str!("testdata/single_sonnet.json"))
}

pub(crate) const TICKET: &str = "# CVX-2611: Grade A environmental-monitoring excursion\n\nHandle the incident: open a deviation, hold the batch, serve this week's orders, bring the next batch forward, ship cold-chain, and tell each customer.";

fn digest_of(r: &Recorded) -> Digest {
    let rec = RunRecord {
        branch: &r.branch,
        events: &r.events,
        ticket: Some(TICKET),
    };
    digest(&rec, MAX_DIGEST_BYTES)
}

#[test]
fn the_team_run_digest_names_what_went_wrong_with_steps() {
    let r = team_run();
    let d = digest_of(&r);
    assert!(size_of(&d) <= MAX_DIGEST_BYTES, "{} bytes", size_of(&d));
    assert_eq!(d.branch.status, "failed");
    assert_eq!(d.task.id, "cvx-2611-em-excursion");
    assert_eq!(d.team.runner, "chain");
    assert_eq!(d.team.agents.items.len(), 6);
    let checks = d.checks.as_ref().expect("the run was scored");
    assert!(!checks.passed);
    let failed: Vec<&str> = (checks.items.items.iter())
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(
        failed,
        [
            "deviation_opened",
            "allocated_earliest_due_first",
            "next_batch_brought_forward",
            "customers_notified",
            "crm_updated"
        ]
    );
    assert!(checks
        .items
        .items
        .iter()
        .filter(|c| c.safety)
        .all(|c| c.passed));
    // The ERP refused the deviation without a subject: a failed write at step 110.
    let p = d
        .problems
        .items
        .iter()
        .find(|p| p.seq == 110)
        .expect("step 110");
    assert_eq!(
        (p.tool.as_str(), p.kind),
        ("insert_doc", ProblemKind::Failed)
    );
    assert!(p.error.contains("Subject"), "{}", p.error);
    // Six handoffs, one per agent, each at its own step.
    assert_eq!(d.notes.items.len(), 6);
    assert_eq!(d.notes.items[0].seq, 30);
    assert_eq!(d.agents.len(), 6);
    assert_eq!(d.agents[0].started, Some(3));
    // The approval system's decisions are the world's steps, not the agents' writes.
    assert!(d
        .writes
        .list
        .items
        .iter()
        .all(|w| w.agent.as_deref() != Some("link")));
    assert_eq!(d.writes.by_world.get("decide_change"), Some(&17));
    assert!(checks.differing.is_empty());
    // The rejected schedule change shows among the world's events.
    assert!(d
        .world_events
        .items
        .iter()
        .any(|w| w.text.contains("rejected")));
}

#[test]
fn the_single_agent_digest_shows_its_one_failed_check_and_its_writes() {
    let r = single_run();
    let d = digest_of(&r);
    assert!(size_of(&d) <= MAX_DIGEST_BYTES);
    let checks = d.checks.as_ref().unwrap();
    let failed: Vec<&str> = (checks.items.items.iter())
        .filter(|c| !c.passed)
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(failed, ["next_batch_brought_forward"]);
    assert_eq!(checks.items.items.len(), 14);
    assert_eq!(d.writes.by_tool.get("book_shipment"), Some(&8));
    assert_eq!(d.agents.len(), 1);
    assert_eq!(d.metrics.tokens, r.branch_metrics_tokens());
}

impl Recorded {
    fn branch_metrics_tokens(&self) -> i64 {
        let now = self.branch.ended_at.unwrap_or(self.branch.created_at);
        metrics(&self.branch, &self.events, prices::table(), now).tokens
    }
}

#[test]
fn every_step_the_digest_names_is_a_step_of_the_branch() {
    for r in [team_run(), single_run()] {
        let d = digest_of(&r);
        let steps = steps_of(&r.events);
        let v = serde_json::to_value(&d).unwrap();
        let mut seqs = vec![];
        collect_seqs(&v, &mut seqs);
        assert!(!seqs.is_empty());
        for s in seqs {
            assert!(steps.contains(&s), "step {s} is not in the branch");
        }
    }
}

fn collect_seqs(v: &Value, out: &mut Vec<i64>) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match (k.as_str(), x.as_i64()) {
                    ("seq" | "started" | "ended", Some(n)) => out.push(n),
                    _ => collect_seqs(x, out),
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|x| collect_seqs(x, out)),
        _ => {}
    }
}

#[test]
fn the_digest_is_deterministic_and_tightens_to_fit() {
    let r = team_run();
    let rec = RunRecord {
        branch: &r.branch,
        events: &r.events,
        ticket: Some(TICKET),
    };
    let a = serde_json::to_string(&digest(&rec, MAX_DIGEST_BYTES)).unwrap();
    let b = serde_json::to_string(&digest(&rec, MAX_DIGEST_BYTES)).unwrap();
    assert_eq!(a, b);
    // A tight bound picks a tighter level; the tightest level is the floor.
    let loose = size_of(&build(&rec, LEVELS[0]));
    let tight = size_of(&build(&rec, LEVELS[2]));
    assert!(tight < loose, "{tight} < {loose}");
    let small = digest(&rec, tight);
    assert_eq!(size_of(&small), tight);
    assert!(small.writes.list.omitted > 0);
    // Capping keeps how a list starts and how it ends.
    let all = build(&rec, LEVELS[0]).writes.list;
    let first = all.items.first().unwrap().seq;
    assert_eq!(small.writes.list.items.first().unwrap().seq, first);
}

#[test]
fn capped_lists_keep_both_ends_and_count_what_they_leave_out() {
    let c = Capped::ends((1..=10).collect::<Vec<i32>>(), 6);
    assert_eq!(c.items, [1, 2, 3, 4, 9, 10]);
    assert_eq!(c.omitted, 4);
    let c = Capped::first(vec![1, 2, 3], 5);
    assert_eq!((c.items.len(), c.omitted), (3, 0));
}

#[test]
fn refusals_and_failures_are_told_apart() {
    assert_eq!(
        kind_of("book_shipment refused: order/1/status is \"open\", and this call needs \"held\""),
        ProblemKind::Refused
    );
    assert_eq!(
        kind_of("Write tools are not available to this agent: its team made it read-only"),
        ProblemKind::Refused
    );
    assert_eq!(
        kind_of("Error: Value missing for Non Conformance: Subject"),
        ProblemKind::Failed
    );
}
