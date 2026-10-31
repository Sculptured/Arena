#[path = "support/fake_upstream.rs"]
mod fake_upstream;

use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, AgentResult, AgentRun, Driver},
    app::{self, AppState, Config},
    exec::BranchCtx,
    model::*,
    runner::{self, StartPoint},
    store::Store,
    tasks::Tasks,
};
use fake_upstream::FakeUpstream;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use world::{
    schema::load_base,
    snapshot::{Source, Worlds},
    testing::{self, Cleanup},
};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

static WORLDS: OnceCell<Arc<Worlds>> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

async fn worlds() -> Arc<Worlds> {
    WORLDS
        .get_or_init(|| async {
            let w = Worlds::new(testing::admin());
            load_base(w.admin(), &data_dir()).await.unwrap();
            Arc::new(w)
        })
        .await
        .clone()
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("fake/model".into()),
        prompt: format!("You are {id}."),
    }
}

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

struct Fail;
impl Driver for Fail {
    fn run<'a>(&'a self, _: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async {
            AgentResult {
                ok: false,
                error: Some("boom".into()),
                tokens: json!({}),
                cost_usd: None,
                cost_source: None,
                seconds: 0.0,
                failure: None,
            }
        })
    }
}

struct Fx {
    state: Arc<AppState>,
    _clean: Cleanup,
}

async fn setup(tag: &str, upstream: Option<&FakeUpstream>, failing: bool) -> Fx {
    let worlds = worlds().await;
    let meta = format!(
        "arena_test_runner_{}_{}",
        tag,
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
    let clean = Cleanup::new();
    clean.meta(&meta);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = Config::new(data_dir());
    if let Some(u) = upstream {
        config.upstream_base = u.base_url.clone();
    }
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = format!("http://127.0.0.1:{port}");
    let state = Arc::new(AppState::new(store, worlds, config));
    let d: Arc<dyn Driver> = if failing {
        Arc::new(Fail)
    } else {
        Arc::new(OpenDriver::new(state.clone()))
    };
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    let app = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fx {
        state,
        _clean: clean,
    }
}

async fn teardown(fx: Fx, branches: &[&str]) {
    let worlds = fx.state.worlds.clone();
    for b in branches {
        worlds.drop_db(&format!("branch_{b}")).await.unwrap();
    }
    drop(fx.state);
}

async fn finished(fx: &Fx, branch: &str) -> Branch {
    for _ in 0..600 {
        let b = fx.state.store.branch(branch).await.unwrap();
        if b.status != BranchStatus::Running {
            return b;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("branch still running");
}

fn tool_msg(calls: Vec<(&str, &str, Value)>) -> Value {
    let tcs: Vec<Value> = calls
        .into_iter()
        .map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}))
        .collect();
    json!({"role": "assistant", "content": null, "tool_calls": tcs})
}

fn text_msg(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn triage_script() -> Vec<Value> {
    vec![
        tool_msg(vec![(
            "t1",
            "find_user_id_by_name_zip",
            json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}),
        )]),
        tool_msg(vec![(
            "t2",
            "get_order_details",
            json!({"order_id": "#W7181492"}),
        )]),
        tool_msg(vec![(
            "t3",
            "handoff",
            json!({"note": "Isabella wants to exchange an item in #W7181492."}),
        )]),
    ]
}

fn gold_write() -> Value {
    let tasks = Tasks::load(&data_dir()).unwrap();
    let t = tasks.get("27").unwrap();
    let a = t["evaluation_criteria"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "exchange_delivered_order_items")
        .unwrap();
    tool_msg(vec![(
        "r1",
        "exchange_delivered_order_items",
        a["arguments"].clone(),
    )])
}

fn wrong_write() -> Value {
    tool_msg(vec![(
        "r1",
        "modify_user_address",
        json!({"user_id": "isabella_johansson_2152", "address1": "1 Wrong St", "address2": "", "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"}),
    )])
}

fn kinds(evs: &[Event]) -> Vec<EventKind> {
    evs.iter()
        .map(|e| e.kind)
        .filter(|k| !matches!(k, EventKind::ModelCall | EventKind::ToolCall))
        .collect()
}

#[test]
fn scripted_team_passes_task_27() {
    run(async {
        let mut script = triage_script();
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("pass", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::Handoff,
                EventKind::AgentEnd,
                EventKind::AgentStart,
                EventKind::AgentEnd,
                EventKind::Check
            ]
        );
        assert_eq!(evs[1].agent.as_deref(), Some("triage"));
        let check: CheckPayload =
            serde_json::from_value(evs.last().unwrap().payload.clone()).unwrap();
        assert!(check.passed);
        assert_eq!(check.differing_rows, Some(vec![]));
        assert_eq!(check.gold_db.as_deref(), Some("gold_27"));
        // The resolver's input holds the ticket, the policy and the triage note.
        let req = &up.requests()[3]["messages"][1]["content"];
        let input = req.as_str().unwrap();
        assert!(input.contains("# Customer ticket") && input.contains("# Store policy"));
        assert!(
            input.contains("# Handoff note from triage")
                && input.contains("exchange an item in #W7181492")
        );
        assert!(fx.state.live.read().unwrap().is_empty());
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn wrong_write_fails_with_differing_rows() {
    run(async {
        let mut script = triage_script();
        script.extend([wrong_write(), text_msg("Done.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("wrong", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error, None);
        let evs = fx.state.store.events(&branch).await.unwrap();
        let check: CheckPayload =
            serde_json::from_value(evs.last().unwrap().payload.clone()).unwrap();
        assert!(!check.passed);
        let rows = check.differing_rows.unwrap();
        assert!(
            rows.contains(&"users/isabella_johansson_2152".to_string()),
            "{rows:?}"
        );
        assert!(rows.contains(&"orders/#W7181492".to_string()), "{rows:?}");
        let mut sorted = rows.clone();
        sorted.sort();
        assert_eq!(rows, sorted);
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn driver_error_is_infra_error_not_failed() {
    run(async {
        let fx = setup("infra", None, true).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError);
        assert_eq!(b.error.as_deref(), Some("boom"));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::AgentEnd
            ]
        );
        let end: AgentEndPayload =
            serde_json::from_value(evs.last().unwrap().payload.clone()).unwrap();
        assert!(!end.ok);
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn model_cutoff_is_failed_not_infra_error() {
    run(async {
        let up = FakeUpstream::start(vec![
            json!({"role": "assistant", "content": "partial", "_finish_reason": "length"}),
        ])
        .await;
        let fx = setup("cutoff", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error.as_deref(), Some("model output was cut off"));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert!(!kinds(&evs).contains(&EventKind::Check));
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn missing_handoff_fails() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("I forgot to hand off.")]).await;
        let fx = setup("nohandoff", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error.as_deref(), Some("no handoff"));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::AgentEnd
            ]
        );
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn run_branch_can_start_mid_chain_from_recorded_notes() {
    run(async {
        let up = FakeUpstream::start(vec![gold_write(), text_msg("Exchanged.")]).await;
        let fx = setup("mid", Some(&up), false).await;
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let branch = fx
            .state
            .store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
        let db = fx
            .state
            .worlds
            .new_branch(&branch, Source::Base)
            .await
            .unwrap();
        let ctx = Arc::new(BranchCtx::new(branch.clone(), db, t, "27".into()));
        let from = StartPoint {
            agent_index: 1,
            resume: None,
            notes: vec!["Carried note.".into()],
        };
        runner::run_branch(fx.state.clone(), ctx, from).await;
        let b = fx.state.store.branch(&branch).await.unwrap();
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![EventKind::AgentStart, EventKind::AgentEnd, EventKind::Check]
        );
        assert!(up.requests()[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Carried note."));
        teardown(fx, &[&branch]).await;
    })
}
