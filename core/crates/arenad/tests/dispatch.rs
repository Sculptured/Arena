//! The dispatch runner: a lead delegates, its team works in parallel, reports back and stays
//! available, and the lead finishes. Without a model: what the API refuses, the teams it
//! suggests, and a lead that can't reach its model. `live_`: a team on a real model (see
//! `support/live.rs`), then a fork of its run.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/live.rs"]
mod live;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock},
};

use app_support::send;
use arenad::{app::AppState, fork, model::*, providers::ProviderConfig, runner};
use live::Live;
use packages::TestHub;
use pg::Cleanup;
use serde_json::json;

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    _hub: TestHub,
    _clean: Cleanup,
}

/// An app on the retail benchmark with the real drivers; `providers` replaces the environment's.
async fn setup(tag: &str, providers: Option<ProviderConfig>) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("dispatch", tag).await;
    let (state, base) = app_support::app_for(store, hub.hub.clone(), Some("retail"), |c| {
        if let Some(p) = providers {
            c.providers = p;
        }
    })
    .await;
    Fx {
        state,
        base,
        _hub: hub,
        _clean: clean,
    }
}

fn spec(id: &str, kind: AgentKind) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind,
        model: Some("m".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
        lead: false,
    }
}

fn team(agents: Vec<AgentSpec>) -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents,
    }
}

fn of_kind(evs: &[Event], kind: EventKind) -> Vec<&Event> {
    evs.iter().filter(|e| e.kind == kind).collect()
}

/// A Codex agent can't keep a session, and a team has one lead: both refused before anything
/// runs. The teams endpoint gives each suggested team with its runner.
#[test]
fn the_api_refuses_what_a_dispatch_run_cant_take_and_lists_the_teams() {
    run(async {
        let fx = setup("api", None).await;
        let http = reqwest::Client::new();
        let post = |t: Team| {
            let body = json!({"task_id": "27", "team": t, "runner": "dispatch"});
            send(http.post(format!("{}/api/runs", fx.base)).json(&body))
        };
        let codex = team(vec![
            spec("lead", AgentKind::ClaudeCode),
            spec("x", AgentKind::Codex),
        ]);
        let (status, body) = post(codex).await;
        assert_eq!(status, 400, "{body}");
        assert!(body["error"].as_str().unwrap().contains("Codex"), "{body}");
        let mut two = team(vec![
            spec("a", AgentKind::ClaudeCode),
            spec("b", AgentKind::ClaudeCode),
        ]);
        two.agents.iter_mut().for_each(|a| a.lead = true);
        let (status, body) = post(two).await;
        assert_eq!(status, 400, "{body}");
        assert!(fx.state.store.runs().await.unwrap().is_empty());
        let (status, teams) = send(http.get(format!("{}/api/teams?model=haiku", fx.base))).await;
        assert_eq!(status, 200);
        let first = &teams[0];
        assert_eq!(first["runner"], "chain");
        assert!(first["agents"][0]["model"] == "haiku", "{first}");
    })
}

/// The lead's first turn fails when its provider can't be reached: the run ends `infra_error`,
/// with the lead's wake and idle recorded and every agent's start and end.
#[test]
fn a_lead_that_cant_reach_its_model_ends_the_run_infra_error() {
    run(async {
        let providers = ProviderConfig::with_upstream(app_support::NOWHERE);
        let fx = setup("infra", Some(providers)).await;
        let t = team(vec![
            spec("lead", AgentKind::Open),
            spec("helper", AgentKind::Open),
        ]);
        let started = runner::start_run_with(&fx.state, "27", t, Runner::Dispatch);
        let (_run, branch) = started.await.unwrap();
        let b = live::ended(&fx.state, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        let error = b.error.unwrap();
        assert!(
            error.starts_with("the lead failed: model call failed (502)"),
            "{error}"
        );
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(evs[0].payload["runner"], "dispatch");
        assert_lifecycle_events(&evs);
        assert_lead_woken_with_its_brief(&evs);
        assert!(fx.state.live.read().unwrap().is_empty());
    })
}

fn assert_lifecycle_events(evs: &[Event]) {
    let kinds: Vec<(EventKind, Option<&str>)> = (evs.iter())
        .filter(|e| {
            !matches!(
                e.kind,
                EventKind::ToolCall | EventKind::ModelCall | EventKind::WorldView
            )
        })
        .map(|e| (e.kind, e.agent.as_deref()))
        .collect();
    let lead = Some("lead");
    let helper = Some("helper");
    assert_eq!(
        kinds,
        [
            (EventKind::RunStart, None),
            (EventKind::AgentStart, lead),
            (EventKind::AgentStart, helper),
            (EventKind::Wake, lead),
            (EventKind::Idle, lead),
            (EventKind::AgentEnd, lead),
            (EventKind::AgentEnd, helper),
        ]
    );
}

fn assert_lead_woken_with_its_brief(evs: &[Event]) {
    let wake = &of_kind(evs, EventKind::Wake)[0].payload;
    assert_eq!(wake["reason"], "start");
    let input = wake["input"].as_str().unwrap();
    assert!(
        input.contains("You lead this run") && input.contains("`helper`"),
        "{input}"
    );
}

#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_lead_dispatches_its_team_works_in_parallel_reports_and_the_lead_finishes() {
    if let Some(l) = Live::any("live_a_lead_dispatches_its_team") {
        run(a_lead_dispatches(l));
    }
}

const LEAD: &str = "You lead a two-agent check of the retail store. In your first turn, call \
`delegate` twice, one right after the other: to `orders` with the task \"Look up the order of the \
ticket with one read tool and report its status in one line\", and to `customer` with the task \"Look \
up the customer of the ticket with one read tool and report their name in one line\". Then end your \
turn. When both have reported, call `finish` with a one-line summary. Change nothing in the world.";

const MEMBER: &str = "You are one agent of a team. Do exactly the task you are given with read \
tools only, then call `report` once with one line. Change nothing in the world.";

async fn a_lead_dispatches(live: Live) {
    let fx = setup("live", None).await;
    let mut lead = live.agent("lead", LEAD);
    lead.lead = true;
    let t = live.team(vec![
        live.agent("orders", MEMBER),
        lead,
        live.agent("customer", MEMBER),
    ]);
    let started = runner::start_run_with(&fx.state, "27", t, Runner::Dispatch);
    let (_run, branch) = started.await.unwrap();
    let (b, evs) = live::ended_ok(&fx.state, &branch).await;
    live::assert_recorded(&b, &evs, &live);
    assert_eq!(evs[0].payload["runner"], "dispatch");
    assert_dispatched(&evs);
    assert_parallel(&evs);
    assert_finished_last(&evs);
    fork_restarts_everyone(&fx, &branch, &evs).await;
}

/// Each delegation woke its agent with it, and each agent that was given work reported to the
/// lead (itself, or arena passing on its turn's last words).
fn assert_dispatched(evs: &[Event]) {
    let delegations = of_kind(evs, EventKind::Delegation);
    assert!(!delegations.is_empty(), "the lead delegated");
    for d in &delegations {
        assert_eq!(d.agent.as_deref(), Some("lead"));
        let to = d.payload["to"].as_str().unwrap();
        let woke = evs.iter().any(|e| {
            e.kind == EventKind::Wake
                && e.agent.as_deref() == Some(to)
                && e.payload["mail"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(d.seq))
        });
        assert!(woke, "delegation {} woke {to}", d.seq);
        let reported = evs.iter().any(|e| {
            e.seq > d.seq && e.kind == EventKind::Report && e.agent.as_deref() == Some(to)
        });
        assert!(reported, "{to} reported after delegation {}", d.seq);
    }
    let reports = of_kind(evs, EventKind::Report);
    assert!(reports.iter().all(|r| r.payload["to"] == "lead"));
}

/// The working spans (wake to idle) of each agent.
fn spans(evs: &[Event]) -> HashMap<String, Vec<(i64, i64)>> {
    let mut open: HashMap<String, i64> = HashMap::new();
    let mut out: HashMap<String, Vec<(i64, i64)>> = HashMap::new();
    for e in evs {
        let Some(a) = e.agent.clone() else { continue };
        match e.kind {
            EventKind::Wake => {
                open.insert(a, e.seq);
            }
            EventKind::Idle | EventKind::AgentEnd => {
                if let Some(s) = open.remove(&a) {
                    out.entry(a).or_default().push((s, e.seq));
                }
            }
            _ => {}
        }
    }
    out
}

/// The two specialists, delegated to in the lead's same turn, worked at the same time.
fn assert_parallel(evs: &[Event]) {
    let s = spans(evs);
    let (Some(o), Some(c)) = (s.get("orders"), s.get("customer")) else {
        panic!("both specialists worked: {s:?}");
    };
    let overlap = o.iter().any(|a| c.iter().any(|b| a.0 < b.1 && b.0 < a.1));
    assert!(overlap, "orders {o:?} and customer {c:?} worked at once");
}

/// The lead's finish is the run's last word: after it only agents' ends (and turns ending),
/// then the check; no tool call lands after it.
fn assert_finished_last(evs: &[Event]) {
    let finish = of_kind(evs, EventKind::Finish);
    assert_eq!(finish.len(), 1, "the lead finished once");
    assert_eq!(finish[0].agent.as_deref(), Some("lead"));
    let after = evs.iter().filter(|e| e.seq > finish[0].seq);
    for e in after {
        let ok = matches!(
            e.kind,
            EventKind::AgentEnd | EventKind::Idle | EventKind::Check | EventKind::ModelCall
        );
        assert!(ok, "{:?} after the finish: {}", e.kind, e.payload);
    }
    assert_eq!(evs.last().unwrap().kind, EventKind::Check);
}

/// A fork at the lead's second delegation restarts every agent there: the lead, working at the
/// fork point, carries on with its history; the run goes on to its end.
async fn fork_restarts_everyone(fx: &Fx, branch: &str, evs: &[Event]) {
    let delegations = of_kind(evs, EventKind::Delegation);
    let at = delegations.get(1).unwrap_or(&delegations[0]).seq;
    let forked = fork::fork(&fx.state, branch, at, HashMap::new())
        .await
        .unwrap();
    assert_eq!(forked.start_seq, at, "a dispatch run forks where asked");
    let b = live::ended(&fx.state, &forked.branch_id).await;
    assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
    let evs = fx.state.store.events(&forked.branch_id).await.unwrap();
    let new: Vec<&Event> = evs.iter().filter(|e| e.seq >= at).collect();
    let lead_wake = new
        .iter()
        .find(|e| e.kind == EventKind::Wake && e.agent.as_deref() == Some("lead"));
    let wake = lead_wake.expect("the lead was woken on the fork");
    assert_eq!(wake.payload["reason"], "resume");
    let input = wake.payload["input"].as_str().unwrap();
    assert!(input.contains("Your work so far"), "{input}");
    if delegations.len() > 1 {
        assert!(
            input.contains("You gave `"),
            "the first delegation is in its history: {input}"
        );
    }
    let ends = new.iter().filter(|e| e.kind == EventKind::AgentEnd).count();
    assert_eq!(ends, 3, "every agent ended on the fork");
}
