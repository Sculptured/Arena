//! The state machine on constructed events, with no agent: delegation, waking, reports, the idle
//! rule, finishing, stopping, pausing and a fork's restart.
use super::*;

const S: Duration = Duration::from_secs(1);

fn limits() -> Limits {
    Limits {
        max_run: 600 * S,
        idle_grace: 10 * S,
        max_idle_wakes: 2,
    }
}

fn board() -> Board {
    let ids: Vec<String> = ["lead", "qa", "supply"].map(String::from).to_vec();
    Board::new(&ids, 0, limits())
}

fn said(seq: i64, from: &str, kind: EventKind, to: Option<&str>, text: &str) -> Input {
    let mut s = Said::new(from, kind, to, text);
    s.seq = seq;
    Input::Said(s)
}

fn delegate(seq: i64, to: &str, task: &str) -> Input {
    said(seq, "lead", EventKind::Delegation, Some(to), task)
}

fn ended(agent: &str, text: &str) -> Input {
    Input::TurnEnded {
        agent: agent.into(),
        ok: true,
        error: None,
        failure: None,
        text: text.into(),
    }
}

fn failed(agent: &str, failure: Option<FailureKind>) -> Input {
    Input::TurnEnded {
        agent: agent.into(),
        ok: false,
        error: Some("boom".into()),
        failure,
        text: String::new(),
    }
}

/// The agents woken by `actions`, with why and the seqs of their mail.
fn wakes(actions: &[Action]) -> Vec<(String, Reason, Vec<i64>)> {
    let wake = |a: &Action| match a {
        Action::Wake {
            agent,
            reason,
            mail,
        } => Some((agent.clone(), *reason, mail.iter().map(|m| m.seq).collect())),
        _ => None,
    };
    actions.iter().filter_map(wake).collect()
}

fn end(actions: &[Action]) -> Option<End> {
    actions.iter().find_map(|a| match a {
        Action::End(e) => Some(e.clone()),
        _ => None,
    })
}

fn wake(agent: &str, reason: Reason, mail: &[i64]) -> (String, Reason, Vec<i64>) {
    (agent.into(), reason, mail.to_vec())
}

#[test]
fn the_lead_starts_alone_and_delegations_wake_specialists_in_parallel() {
    let mut b = board();
    assert_eq!(wakes(&b.start()), [wake("lead", Reason::Start, &[])]);
    let a = b.step(delegate(5, "qa", "hold the batch"), S);
    assert_eq!(wakes(&a), [wake("qa", Reason::Mail, &[5])]);
    let a = b.step(delegate(6, "supply", "allocate"), S);
    assert_eq!(
        wakes(&a),
        [wake("supply", Reason::Mail, &[6])],
        "both work at once"
    );
    let working = b
        .statuses()
        .iter()
        .filter(|s| s.state == State::Working)
        .count();
    assert_eq!(working, 3);
}

#[test]
fn work_for_a_busy_agent_waits_and_comes_in_one_turn_when_it_is_idle() {
    let mut b = board();
    b.start();
    b.step(delegate(5, "qa", "first"), S);
    assert!(wakes(&b.step(delegate(7, "qa", "second"), S)).is_empty());
    let m = said(8, "supply", EventKind::Message, Some("qa"), "fyi");
    assert!(wakes(&b.step(m, S)).is_empty());
    let a = b.step(said(9, "qa", EventKind::Report, Some("lead"), "held"), S);
    assert!(wakes(&a).is_empty(), "the lead is still working");
    let a = b.step(ended("qa", "done"), S);
    assert_eq!(wakes(&a), [wake("qa", Reason::Mail, &[7, 8])]);
    assert!(
        !a.iter().any(|x| matches!(x, Action::PassOn { .. })),
        "qa reported"
    );
}

#[test]
fn a_report_wakes_the_idle_lead_and_a_silent_turn_end_is_passed_on() {
    let mut b = board();
    b.start();
    b.step(delegate(5, "qa", "hold"), S);
    b.step(delegate(6, "supply", "allocate"), S);
    assert!(wakes(&b.step(ended("lead", "waiting"), S)).is_empty());
    let a = b.step(said(9, "qa", EventKind::Report, Some("lead"), "held"), S);
    assert_eq!(wakes(&a), [wake("lead", Reason::Mail, &[9])]);
    let a = b.step(ended("supply", "allocated 3 lots"), S);
    let pass = Action::PassOn {
        agent: "supply".into(),
        text: "allocated 3 lots".into(),
    };
    assert_eq!(a, [pass]);
    let a = b.step(ended("qa", ""), S);
    assert!(
        a.is_empty(),
        "qa reported during its turn: nothing to pass on"
    );
}

#[test]
fn the_lead_alone_ends_the_run_and_nothing_happens_after() {
    let mut b = board();
    b.start();
    b.step(delegate(5, "qa", "hold"), S);
    let a = b.step(said(9, "lead", EventKind::Finish, None, "done"), S);
    assert_eq!(end(&a), Some(End::Finished));
    assert!(b.step(delegate(10, "supply", "late"), S).is_empty());
    assert!(b.step(Input::Tick, 1000 * S).is_empty());
}

#[test]
fn an_idle_team_wakes_the_lead_after_the_grace_then_fails() {
    let mut b = board();
    b.start();
    b.step(ended("lead", "nothing to do"), S);
    assert!(b.step(Input::Tick, 2 * S).is_empty(), "the grace starts");
    assert!(
        b.step(Input::Tick, 11 * S).is_empty(),
        "still within the grace"
    );
    let a = b.step(Input::Tick, 12 * S);
    assert_eq!(wakes(&a), [wake("lead", Reason::Idle(1), &[])]);
    assert!(
        b.step(Input::Tick, 30 * S).is_empty(),
        "the lead is working"
    );
    b.step(ended("lead", "still nothing"), 31 * S);
    b.step(Input::Tick, 32 * S);
    let a = b.step(Input::Tick, 42 * S);
    assert_eq!(wakes(&a), [wake("lead", Reason::Idle(2), &[])]);
    b.step(ended("lead", "hm"), 43 * S);
    b.step(Input::Tick, 44 * S);
    let a = b.step(Input::Tick, 54 * S);
    assert!(matches!(end(&a), Some(End::Failed(why)) if why.contains("woken 2 times")));
}

#[test]
fn mail_or_work_resets_the_idle_grace() {
    let mut b = board();
    b.start();
    b.step(ended("lead", "x"), S);
    b.step(Input::Tick, 2 * S);
    b.step(
        said(4, "qa", EventKind::Message, Some("lead"), "a question"),
        5 * S,
    );
    b.step(ended("lead", "answered"), 6 * S);
    assert!(
        b.step(Input::Tick, 13 * S).is_empty(),
        "the grace started again at 13 s"
    );
    assert!(!wakes(&b.step(Input::Tick, 23 * S)).is_empty());
}

#[test]
fn the_time_limit_ends_the_run_failed_and_leaves_out_pauses() {
    let mut b = board();
    b.start();
    b.step(Input::Pause, 100 * S);
    assert!(
        b.step(Input::Tick, 650 * S).is_empty(),
        "paused: no limit, no idle rule"
    );
    b.step(Input::Resume, 650 * S);
    assert!(
        b.step(Input::Tick, 1149 * S).is_empty(),
        "550 s were paused"
    );
    let a = b.step(Input::Tick, 1150 * S);
    assert!(matches!(end(&a), Some(End::Failed(why)) if why.contains("time limit")));
}

#[test]
fn mail_waits_while_paused_and_is_delivered_on_resume() {
    let mut b = board();
    b.start();
    b.step(Input::Pause, S);
    assert!(wakes(&b.step(delegate(5, "qa", "hold"), 2 * S)).is_empty());
    assert!(wakes(&b.step(ended("lead", "w"), 3 * S)).is_empty());
    let a = b.step(Input::Resume, 9 * S);
    assert_eq!(wakes(&a), [wake("qa", Reason::Mail, &[5])]);
}

#[test]
fn stop_ends_the_run_unscored() {
    let mut b = board();
    b.start();
    assert_eq!(end(&b.step(Input::Stop, S)), Some(End::Stopped));
}

#[test]
fn a_failed_specialist_is_reported_to_the_lead_and_gets_no_more_work() {
    let mut b = board();
    b.start();
    b.step(delegate(5, "qa", "hold"), S);
    b.step(ended("lead", "w"), S);
    let a = b.step(failed("qa", None), S);
    let (who, reason, _) = &wakes(&a)[0];
    assert_eq!((who.as_str(), *reason), ("lead", Reason::Mail));
    let notice = match &a[0] {
        Action::Wake { mail, .. } => mail[0].clone(),
        _ => unreachable!(),
    };
    assert_eq!(notice.kind, MailKind::Notice);
    assert!(notice.text.contains("qa stopped working: boom"));
    b.step(ended("lead", "ok"), S);
    let a = b.step(delegate(12, "qa", "again"), S);
    let mail = match &a[0] {
        Action::Wake { agent, mail, .. } if agent == "lead" => mail.clone(),
        other => panic!("{other:?}"),
    };
    assert!(mail[0]
        .text
        .contains("delegation at step 12 was not delivered"));
}

#[test]
fn the_leads_failure_ends_the_run_as_the_model_or_the_harness_failed() {
    let mut b = board();
    b.start();
    let a = b.step(failed("lead", Some(FailureKind::Model)), S);
    assert!(matches!(end(&a), Some(End::Failed(why)) if why == "the lead failed: boom"));
    let mut b = board();
    b.start();
    assert!(matches!(
        end(&b.step(failed("lead", None), S)),
        Some(End::Broken(_))
    ));
}

#[test]
fn a_fork_restarts_who_was_working_and_delivers_what_was_waiting() {
    let mut b = board();
    let pending = vec![(
        "supply".to_string(),
        Mail {
            seq: 14,
            from: "lead".into(),
            kind: MailKind::Delegation,
            text: "allocate".into(),
        },
    )];
    let a = b.resume(1, pending, &["qa".to_string()]);
    assert_eq!(
        wakes(&a),
        [
            wake("qa", Reason::Resume, &[]),
            wake("supply", Reason::Mail, &[14])
        ]
    );
    b.step(ended("qa", "x"), S);
    b.step(ended("supply", "y"), S);
    b.step(Input::Tick, 2 * S);
    let a = b.step(Input::Tick, 12 * S);
    assert_eq!(
        wakes(&a),
        [wake("lead", Reason::Idle(2), &[])],
        "one idle wake was used before the fork"
    );
}
