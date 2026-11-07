//! What a run's saves cost and how they go: deleting a run, the saves limit, the cleanup at
//! start. `live_`: runs on a real model make the saves (see `support/live.rs`).
#[path = "support/app.rs"]
mod app_support;
#[path = "support/live.rs"]
mod live;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use arenad::{
    app::{AppState, Config},
    model::*,
};
use live::Live;
use packages::TestHub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

/// Runs `body` on the live model, when there is one (else the test was skipped, said why).
fn run_live<F: Future<Output = ()>>(live: Option<Live>, body: impl FnOnce(Live) -> F) {
    if let Some(live) = live {
        run(body(live));
    }
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("m".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
    }
}

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

struct Fx {
    state: Arc<AppState>,
    _hub: TestHub,
    base: String,
    http: reqwest::Client,
    _clean: Cleanup,
}

/// An app that runs no agent; `tweak` adjusts its config.
async fn setup_with(tag: &str, tweak: impl FnOnce(&mut Config)) -> Fx {
    setup_on(tag, false, tweak).await
}

/// An app on the configured providers and Claude Code, for live tests.
async fn live_setup(tag: &str) -> Fx {
    setup_on(tag, true, |_| {}).await
}

async fn setup_on(tag: &str, live: bool, tweak: impl FnOnce(&mut Config)) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("disk", tag).await;
    let (state, base) = match live {
        true => app_support::app_for(store, hub.hub.clone(), Some("retail"), tweak).await,
        false => app_support::idle_app(store, hub.hub.clone(), None, tweak).await,
    };
    Fx {
        state,
        _hub: hub,
        base,
        http: reqwest::Client::new(),
        _clean: clean,
    }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(self.http.get(format!("{}{path}", self.base))).await
    }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let req = self.http.post(format!("{}{path}", self.base)).json(&body);
        app_support::send(req).await
    }
    async fn delete(&self, path: &str) -> (u16, Value) {
        app_support::send(self.http.delete(format!("{}{path}", self.base))).await
    }
}

fn solo(live: &Live) -> Team {
    let prompt = "You are a retail support agent. Read the order with get_order_details, then do what the ticket asks.";
    live.team(vec![live.agent("resolver", prompt)])
}

/// A finished solo run of task 27 on `live`: its run and branch ids.
async fn live_run(fx: &Fx, live: &Live) -> (String, String) {
    let (run_id, b) = arenad::runner::start_run(&fx.state, "27", solo(live))
        .await
        .unwrap();
    live::ended_ok(&fx.state, &b).await;
    (run_id, b)
}

/// Every save a branch's events name, except the shared base and gold saves.
async fn run_saves(fx: &Fx, branch: &str) -> Vec<String> {
    let mut out = vec![];
    for e in fx.state.store.events(branch).await.unwrap() {
        for key in ["saves", "final_saves"] {
            if let Some(m) = e.payload[key].as_object() {
                out.extend(m.values().filter_map(Value::as_str).map(str::to_string));
            }
        }
    }
    out.retain(|s| !s.starts_with("base_") && !s.starts_with("gold_"));
    out.sort();
    out.dedup();
    out
}

/// True when the postgres integration still has `save`.
async fn save_exists(fx: &Fx, save: &str) -> bool {
    let c = fx.state.hub.client("postgres").await.unwrap();
    let r = c
        .call::<Value>(
            "inspect",
            json!({"target": {"save": save}, "keys": ["users/x"]}),
        )
        .await;
    match r {
        Ok(_) => true,
        Err(parts::CallError::Part { code: -32002, .. }) => false,
        Err(e) => panic!("inspect {save}: {e}"),
    }
}

async fn disk_bytes(fx: &Fx) -> i64 {
    let (_, h) = fx.get("/api/health").await;
    h["disk"]["bytes"].as_i64().unwrap()
}

/// The base and gold saves of the retail world (part `retail`).
fn shared_saves() -> [&'static str; 2] {
    ["base_retail_0_1_0_retail", "gold_retail_0_1_0_27_retail"]
}

/// Asserts that each of `saves` exists (or not).
async fn assert_saves(fx: &Fx, saves: &[String], exist: bool) {
    for s in saves {
        assert_eq!(save_exists(fx, s).await, exist, "{s}");
    }
}

/// A live run forked at its agent's start (the child runs again); `(run, saves)` with the saves
/// both branches' events name.
async fn run_with_fork(fx: &Fx, live: &Live) -> (String, Vec<String>) {
    let (run_id, b) = live_run(fx, live).await;
    let evs = fx.state.store.events(&b).await.unwrap();
    let at = live::first_of(&evs, EventKind::AgentStart, "resolver").unwrap();
    let child = arenad::fork::fork(&fx.state, &b, at, Default::default())
        .await
        .unwrap()
        .branch_id;
    live::ended_ok(&fx.state, &child).await;
    let mut saves = run_saves(fx, &b).await;
    saves.extend(run_saves(fx, &child).await);
    saves.sort();
    saves.dedup();
    (run_id, saves)
}

/// Deleting a run drops every save its branches' events name, and only those: the base and gold
/// saves stay, and the arena's disk use falls.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_delete_run_drops_all_part_saves() {
    run_live(
        Live::any("live_delete_run_drops_all_part_saves"),
        delete_run_drops_all_part_saves,
    );
}

async fn delete_run_drops_all_part_saves(live: Live) {
    let fx = live_setup("delete").await;
    let (run_id, saves) = run_with_fork(&fx, &live).await;
    assert_saves(&fx, &saves, true).await;
    let before = disk_bytes(&fx).await;
    let (st, v) = fx.delete(&format!("/api/runs/{run_id}")).await;
    assert_eq!(
        (st, v["dropped"].as_u64()),
        (200, Some(saves.len() as u64)),
        "{v}"
    );
    assert_saves(&fx, &saves, false).await;
    assert_saves(&fx, &shared_saves().map(String::from), true).await;
    let after = disk_bytes(&fx).await;
    if !saves.is_empty() {
        assert!(after < before, "disk usage falls: {before} -> {after}");
    }
    assert!(fx.state.store.runs().await.unwrap().is_empty());
    assert_eq!(fx.delete(&format!("/api/runs/{run_id}")).await.0, 404);
}

/// A run of task 27 with one root branch that has not ended.
async fn new_branch(fx: &Fx) -> (String, String) {
    let t = team();
    let run_id = fx.state.store.create_run("27", &t).await.unwrap();
    let b = fx
        .state
        .store
        .create_branch(&run_id, None, &t, &json!([]), None)
        .await
        .unwrap();
    (run_id, b)
}

/// With the limit exceeded, forking a missing branch is 404 and a running one is 409, and only a
/// finished branch gets the 507. Leaves one root branch behind.
async fn assert_fork_errors_before_limit(fx: &Fx) {
    assert_eq!(
        fx.post("/api/branches/whatever/fork", json!({"at_seq": 1}))
            .await
            .0,
        404
    );
    let (_, b) = new_branch(fx).await;
    assert_eq!(
        fx.post(&format!("/api/branches/{b}/fork"), json!({"at_seq": 1}))
            .await
            .0,
        409
    );
    fx.state
        .store
        .set_status(&b, BranchStatus::Passed, None)
        .await
        .unwrap();
    let (st, v) = fx
        .post(&format!("/api/branches/{b}/fork"), json!({"at_seq": 1}))
        .await;
    assert_over_limit(st, &v);
}

/// The response is the 507 the arena gives when its saves use more than the limit.
fn assert_over_limit(status: u16, body: &Value) {
    assert_eq!(
        (status, body["error"].as_str()),
        (
            507,
            Some("the arena's saves use more than the limit; delete old runs")
        )
    );
}

#[test]
fn delete_running_run_is_409() {
    run(async {
        let fx = setup_with("running", |_| {}).await;
        let (run_id, b) = new_branch(&fx).await;
        let (st, v) = fx.delete(&format!("/api/runs/{run_id}")).await;
        assert_eq!(st, 409, "{v}");
        assert!(
            fx.state.store.find_branch(&b).await.unwrap().is_some(),
            "nothing was deleted"
        );
    })
}

#[test]
fn limit_refuses_new_runs() {
    run(async {
        let fx = setup_with("limit", |c| c.max_db_bytes = 1).await;
        // The base save is the first thing the integration keeps.
        let bench = fx.state.bench().await.unwrap();
        arenad::world::ensure_base(&fx.state.hub, &bench)
            .await
            .unwrap();
        let (_, h) = fx.get("/api/health").await;
        assert_eq!(h["disk"]["limit_bytes"], 1);
        assert!(h["disk"]["bytes"].as_i64().unwrap() > 1, "{h}");
        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_over_limit(st, &v);
        assert_fork_errors_before_limit(&fx).await;
        assert_eq!(
            fx.state.store.runs().await.unwrap()[0].branches.len(),
            1,
            "no branch was created"
        );
    })
}

/// Makes the save `save` from the base save, which no event names.
async fn orphan_save(fx: &Fx, save: &str) {
    let c = fx.state.hub.client("postgres").await.unwrap();
    let start = json!({"instance": "orphan", "config": {}, "seed": 0, "clock_ms": 0,
                       "from": {"save": shared_saves()[0], "mode": "exact"}});
    c.call::<Value>("start", start).await.unwrap();
    let dir = fx.state.hub.save_dir("postgres", save);
    let params = json!({"instance": "orphan", "save": save, "dir": dir});
    c.call::<Value>("save", params).await.unwrap();
    let stop = json!({"instance": "orphan"});
    c.call::<Value>("stop", stop).await.unwrap();
}

/// The cleanup at start drops a save no event names, with its folder, and keeps every save a
/// run's events name and the shared ones.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_startup_gc_keeps_named_saves() {
    run_live(
        Live::any("live_startup_gc_keeps_named_saves"),
        startup_gc_keeps_named_saves,
    );
}

async fn startup_gc_keeps_named_saves(live: Live) {
    let fx = live_setup("gc").await;
    let (_, b) = live_run(&fx, &live).await;
    let mut kept = run_saves(&fx, &b).await;
    kept.extend(shared_saves().map(String::from));
    orphan_save(&fx, "s_orphan").await;
    let folder = |s: &str| fx._hub.saves.join("postgres").join(s);
    assert!(save_exists(&fx, "s_orphan").await && folder("s_orphan").is_dir());

    let r = arenad::gc::run_startup_gc(&fx.state.store, &fx.state.hub, "retail")
        .await
        .unwrap();
    assert!(r.dropped >= 1 && r.bytes > 0, "{r:?}");
    assert!(!save_exists(&fx, "s_orphan").await, "the orphan is dropped");
    assert!(!folder("s_orphan").exists(), "with its folder");
    assert_saves(&fx, &kept, true).await;
    assert!(kept.iter().all(|s| folder(s).is_dir()), "kept folders stay");
}

/// A run whose only branch failed before recording anything names no saves and no world: it is
/// deleted without asking any integration or benchmark (this hub has none).
#[test]
fn delete_run_without_events_needs_no_benchmark() {
    run(async {
        let saves =
            std::env::temp_dir().join(format!("arenad-saves-t8empty{}", std::process::id()));
        let registry = parts::Registry::scan(&[]).unwrap();
        let hub = Arc::new(parts::Hub::new(registry, packages::bin_dir(), saves));
        let (store, _clean) = app_support::meta_store("disk", "noevents").await;
        let (state, base) = app_support::idle_app(store, hub, None, |_| {}).await;
        let run_id = state.store.create_run("27", &team()).await.unwrap();
        let b = state
            .store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
        let failed = Some("setup failed".to_string());
        state
            .store
            .set_status(&b, BranchStatus::InfraError, failed)
            .await
            .unwrap();
        let req = reqwest::Client::new().delete(format!("{base}/api/runs/{run_id}"));
        let (st, v) = app_support::send(req).await;
        assert_eq!((st, v["dropped"].as_u64()), (200, Some(0)), "{v}");
        assert!(state.store.runs().await.unwrap().is_empty());
    })
}
