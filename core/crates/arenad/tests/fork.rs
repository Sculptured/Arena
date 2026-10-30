#[path = "support/fake_upstream.rs"]
mod fake_upstream;

use std::{
    collections::HashMap,
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{self, AppState, Config},
    fork::{self, ForkError},
    model::*,
    runner,
    store::Store,
    tasks::Tasks,
};
use fake_upstream::FakeUpstream;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use world::{schema::load_base, snapshot::Worlds, testing::{self, Cleanup}};

static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

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
    AgentSpec { id: id.into(), name: id.into(), kind: AgentKind::Open, model: Some("fake/model".into()), prompt: format!("You are {id}.") }
}

fn team() -> Team {
    Team { id: "t".into(), name: "T".into(), agents: vec![agent("triage"), agent("resolver")] }
}

struct Fx {
    state: Arc<AppState>,
    _clean: Cleanup,
}

async fn setup(tag: &str, upstream: &FakeUpstream) -> Fx {
    let worlds = worlds().await;
    let meta = format!("arena_test_fork_{}_{}", tag, &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let clean = Cleanup::new();
    clean.meta(&meta);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = Config::new(data_dir());
    config.upstream_base = upstream.base_url.clone();
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = format!("http://127.0.0.1:{port}");
    let state = Arc::new(AppState::new(store, worlds, config));
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    let app = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fx { state, _clean: clean }
}

/// Drops the branches' databases, the snapshots their events name, and the metadata database.
async fn teardown(fx: Fx, branches: &[&str]) {
    let worlds = fx.state.worlds.clone();
    let mut snaps = vec![];
    for b in branches {
        for e in fx.state.store.events(b).await.unwrap() {
            if let Some(s) = e.payload.get("snapshot").and_then(Value::as_str) {
                snaps.push(s.to_string());
            }
        }
    }
    snaps.sort();
    snaps.dedup();
    for s in snaps {
        worlds.drop_db(&s).await.unwrap();
    }
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

const TRIAGE_NOTE: &str = "Isabella wants to exchange an item in #W7181492.";
const ORDER: &str = "#W7181492";
const USER: &str = "isabella_johansson_2152";

fn triage_script() -> Vec<Value> {
    vec![
        tool_msg(vec![("t1", "find_user_id_by_name_zip", json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}))]),
        tool_msg(vec![("t2", "get_order_details", json!({"order_id": ORDER}))]),
        tool_msg(vec![("t3", "handoff", json!({"note": TRIAGE_NOTE}))]),
    ]
}

fn read_order(id: &str) -> Value {
    tool_msg(vec![(id, "get_order_details", json!({"order_id": ORDER}))])
}

fn read_user(id: &str) -> Value {
    tool_msg(vec![(id, "get_user_details", json!({"user_id": USER}))])
}

fn gold_write() -> Value {
    let tasks = Tasks::load(&data_dir()).unwrap();
    let t = tasks.get("27").unwrap();
    let a = t["evaluation_criteria"]["actions"].as_array().unwrap().iter().find(|a| a["name"] == "exchange_delivered_order_items").unwrap();
    tool_msg(vec![("w1", "exchange_delivered_order_items", a["arguments"].clone())])
}

fn wrong_write() -> Value {
    tool_msg(vec![(
        "w2",
        "modify_user_address",
        json!({"user_id": USER, "address1": "1 Wrong St", "address2": "", "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"}),
    )])
}

/// Seq of the `agent`'s ModelCall whose reply calls `tool` (first match).
fn model_call_choosing(evs: &[Event], agent: &str, tool: &str) -> i64 {
    evs.iter()
        .find(|e| {
            e.kind == EventKind::ModelCall
                && e.agent.as_deref() == Some(agent)
                && e.payload["response"]["choices"][0]["message"]["tool_calls"][0]["function"]["name"] == tool
        })
        .unwrap_or_else(|| panic!("no model call by {agent} choosing {tool}"))
        .seq
}

fn agent_start(evs: &[Event], agent: &str) -> i64 {
    evs.iter().find(|e| e.kind == EventKind::AgentStart && e.agent.as_deref() == Some(agent)).unwrap().seq
}

/// Results of the `name` tool calls on `branch` with `seq >= from`, in order.
fn tool_results(evs: &[Event], name: &str, from: i64) -> Vec<Value> {
    evs.iter()
        .filter(|e| e.seq >= from && e.kind == EventKind::ToolCall && e.payload["name"] == name)
        .map(|e| e.payload["result"].clone())
        .collect()
}

fn system_prompt(req: &Value) -> String {
    req["messages"][0]["content"].as_str().unwrap_or("").to_string()
}

async fn order_status_in(fx: &Fx, branch: &str) -> String {
    let out = fx
        .state
        .worlds
        .with_client(&format!("branch_{branch}"), async |c| world::tools::call(c, "get_order_details", &json!({"order_id": ORDER})).await)
        .await
        .unwrap();
    out.result.unwrap()["status"].as_str().unwrap().to_string()
}

#[test]
fn fork_sees_world_at_fork_step_not_after() {
    run(async {
        // Run 1: the resolver reads the order, exchanges it (the write), and finishes.
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), text_msg("Exchanged.")]);
        // The fork resumes at the write's model call: it reads the order first, then writes again.
        script.extend([read_order("f1"), gold_write(), text_msg("Exchanged again.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("world", &up).await;
        let (_run, parent) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &parent).await;
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        assert_eq!(order_status_in(&fx, &parent).await, "exchange requested");
        let pevs = fx.state.store.events(&parent).await.unwrap();
        let at = model_call_choosing(&pevs, "resolver", "exchange_delivered_order_items");

        let resp = fork::fork(&fx.state, &parent, at, HashMap::new()).await.unwrap();
        assert_eq!((resp.start_seq, resp.moved_from, resp.reason.clone()), (at, None, None));
        let child = resp.branch_id.clone();
        let c = finished(&fx, &child).await;
        // The exchange only succeeds again because the world was restored to before it.
        assert_eq!(c.status, BranchStatus::Passed, "{:?}", c.error);
        assert_eq!(c.parent_id.as_deref(), Some(parent.as_str()));
        assert_eq!(c.start_seq, Some(at));
        assert_eq!(c.run_id, b.run_id);

        let cevs = fx.state.store.events(&child).await.unwrap();
        // The child's timeline starts with the parent's events before the fork point.
        let prefix: Vec<_> = cevs.iter().filter(|e| e.seq < at).map(|e| (e.seq, e.kind, e.payload.clone())).collect();
        let pprefix: Vec<_> = pevs.iter().filter(|e| e.seq < at).map(|e| (e.seq, e.kind, e.payload.clone())).collect();
        assert_eq!(prefix, pprefix);
        // The child's first resolver read shows the order as it was before the write.
        let reads = tool_results(&cevs, "get_order_details", at);
        assert_eq!(reads[0]["status"], "delivered", "{}", reads[0]);
        // The resumed model call replays the recorded conversation up to the write.
        let recorded = &pevs.iter().find(|e| e.seq == at).unwrap().payload["request"]["messages"];
        let reqs = up.requests();
        assert_eq!(&reqs[6]["messages"], recorded);
        teardown(fx, &[&parent, &child]).await;
    })
}

#[test]
fn fork_restores_mid_run_snapshot() {
    run(async {
        // Run 1: the resolver exchanges (snapshot 1), then makes a wrong address write
        // (snapshot 2): failed.
        let mut script = triage_script();
        script.extend([gold_write(), wrong_write(), text_msg("Done.")]);
        // The fork redoes the wrong write's step: it reads instead, and stops.
        script.extend([read_user("f1"), read_order("f2"), text_msg("Done right.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("snap", &up).await;
        let (_run, parent) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        assert_eq!(finished(&fx, &parent).await.status, BranchStatus::Failed);
        let pevs = fx.state.store.events(&parent).await.unwrap();
        let at = model_call_choosing(&pevs, "resolver", "modify_user_address");

        let resp = fork::fork(&fx.state, &parent, at, HashMap::new()).await.unwrap();
        let child = resp.branch_id.clone();
        let c = finished(&fx, &child).await;
        assert_eq!(c.status, BranchStatus::Passed, "{:?}", c.error);
        let cevs = fx.state.store.events(&child).await.unwrap();
        // The world at the fork step has the earlier write but not the later one.
        let user = &tool_results(&cevs, "get_user_details", at)[0];
        assert_ne!(user["address"]["address1"], "1 Wrong St", "{user}");
        assert_eq!(tool_results(&cevs, "get_order_details", at)[0]["status"], "exchange requested");
        teardown(fx, &[&parent, &child]).await;
    })
}

#[test]
fn fork_reuses_recorded_handoff() {
    run(async {
        let mut script = triage_script();
        script.extend([wrong_write(), text_msg("Done.")]);
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("handoff", &up).await;
        let (_run, parent) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        assert_eq!(finished(&fx, &parent).await.status, BranchStatus::Failed);
        let pevs = fx.state.store.events(&parent).await.unwrap();
        let before = up.requests().len();
        assert_eq!(before, 5);
        let at = agent_start(&pevs, "resolver");

        let resp = fork::fork(&fx.state, &parent, at, HashMap::new()).await.unwrap();
        assert_eq!((resp.start_seq, resp.moved_from), (at, None));
        let child = resp.branch_id.clone();
        let c = finished(&fx, &child).await;
        assert_eq!(c.status, BranchStatus::Passed, "{:?}", c.error);

        // Triage was not called again: no fork request carries triage's system prompt.
        let reqs = up.requests();
        assert_eq!(reqs.len(), before + 2);
        for r in &reqs[before..] {
            assert_eq!(system_prompt(r), "You are resolver.");
        }
        // The resolver started from the recorded handoff note.
        let input = reqs[before]["messages"][1]["content"].as_str().unwrap();
        assert!(input.contains("# Handoff note from triage") && input.contains(TRIAGE_NOTE), "{input}");
        // No new triage events; the recorded handoff is inherited in the prefix.
        let cevs = fx.state.store.events(&child).await.unwrap();
        assert!(cevs.iter().filter(|e| e.seq >= at).all(|e| e.agent.as_deref() != Some("triage")));
        assert!(cevs.iter().any(|e| e.seq < at && e.kind == EventKind::Handoff && e.payload["note"] == TRIAGE_NOTE));
        teardown(fx, &[&parent, &child]).await;
    })
}

#[test]
fn fork_applies_prompt_change() {
    run(async {
        let mut script = triage_script();
        script.extend([wrong_write(), text_msg("Done.")]);
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("prompt", &up).await;
        let (_run, parent) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        assert_eq!(finished(&fx, &parent).await.status, BranchStatus::Failed);
        let pevs = fx.state.store.events(&parent).await.unwrap();
        let at = model_call_choosing(&pevs, "resolver", "modify_user_address");
        let before = up.requests().len();

        let new_prompt = "You are a careful resolver. Never change addresses.";
        let changes: HashMap<String, Change> =
            [("resolver".to_string(), Change { prompt: Some(new_prompt.into()), ..Default::default() })].into_iter().collect();
        let resp = fork::fork(&fx.state, &parent, at, changes).await.unwrap();
        assert_eq!((resp.start_seq, resp.moved_from), (at, None));
        let child = resp.branch_id.clone();
        let c = finished(&fx, &child).await;
        assert_eq!(c.status, BranchStatus::Passed, "{:?}", c.error);

        // Every fork request's system message is the new prompt, including the resumed one.
        let reqs = up.requests();
        assert_eq!(reqs.len(), before + 2);
        for r in &reqs[before..] {
            assert_eq!(system_prompt(r), new_prompt);
        }
        // The resumed conversation is otherwise the recorded one.
        let recorded = pevs.iter().find(|e| e.seq == at).unwrap().payload["request"]["messages"].as_array().unwrap().clone();
        assert_eq!(reqs[before]["messages"].as_array().unwrap()[1..], recorded[1..]);
        // The branch stores its effective team and the changes as given.
        assert_eq!(c.team.agents[1].prompt, new_prompt);
        assert_eq!(c.team.agents[0].prompt, "You are triage.");
        assert_eq!(c.changes["resolver"]["prompt"], new_prompt);
        assert_eq!(c.fork_note, None);
        teardown(fx, &[&parent, &child]).await;
    })
}

#[test]
fn fork_while_running_is_409() {
    run(async {
        let up = FakeUpstream::start(vec![]).await;
        let fx = setup("running", &up).await;
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let branch = fx.state.store.create_branch(&run_id, None, &t, &json!([]), None).await.unwrap();
        fx.state.store.append(&branch, None, EventKind::RunStart, json!({"task_id": "27"})).await.unwrap();
        assert_eq!(fx.state.store.branch(&branch).await.unwrap().status, BranchStatus::Running);

        let e = fork::fork(&fx.state, &branch, 1, HashMap::new()).await.unwrap_err();
        assert!(matches!(e, ForkError::BranchRunning), "{e:?}");
        assert_eq!(e.to_string(), "the branch is still running");
        // Checked before the plan: even an unknown seq reports the running branch.
        let e = fork::fork(&fx.state, &branch, 999, HashMap::new()).await.unwrap_err();
        assert!(matches!(e, ForkError::BranchRunning), "{e:?}");
        // No branch was created.
        let runs = fx.state.store.runs().await.unwrap();
        assert_eq!(runs[0].branches.len(), 1);
        assert!(up.requests().is_empty());
        drop(fx.state);
    })
}

#[test]
fn fork_of_unknown_branch_is_unknown_branch() {
    run(async {
        let up = FakeUpstream::start(vec![]).await;
        let fx = setup("nobranch", &up).await;
        let e = fork::fork(&fx.state, "nosuchbranch", 1, HashMap::new()).await.unwrap_err();
        assert!(matches!(e, ForkError::UnknownBranch(ref b) if b == "nosuchbranch"), "{e:?}");
        assert!(e.to_string().contains("nosuchbranch"));
        assert!(fx.state.store.find_branch("nosuchbranch").await.unwrap().is_none());
        drop(fx.state);
    })
}

#[test]
fn fork_plan_errors_create_no_branch() {
    run(async {
        let mut script = triage_script();
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("errs", &up).await;
        let (_run, parent) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        finished(&fx, &parent).await;
        let e = fork::fork(&fx.state, &parent, 999, HashMap::new()).await.unwrap_err();
        assert!(matches!(e, ForkError::UnknownSeq), "{e:?}");
        let changes: HashMap<String, Change> =
            [("ghost".to_string(), Change { model: Some("x/y".into()), ..Default::default() })].into_iter().collect();
        let e = fork::fork(&fx.state, &parent, 2, changes).await.unwrap_err();
        assert!(matches!(e, ForkError::UnknownAgent(ref a) if a == "ghost"), "{e:?}");
        assert_eq!(fx.state.store.runs().await.unwrap()[0].branches.len(), 1);
        teardown(fx, &[&parent]).await;
    })
}
