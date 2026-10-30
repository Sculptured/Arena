#[path = "support/fake_upstream.rs"]
mod fake_upstream;

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{self, AppState, Config},
    model::*,
    store::Store,
    tasks::Tasks,
};
use fake_upstream::FakeUpstream;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use world::{schema::load_base, snapshot::{Source, Worlds}, testing::{self, Cleanup}};

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
    base: String,
    http: reqwest::Client,
    clean: Cleanup,
}

async fn setup_with(tag: &str, upstream: &FakeUpstream, tweak: impl FnOnce(&mut Config)) -> Fx {
    let worlds = worlds().await;
    let meta = format!("arena_test_disk_{}_{}", tag, &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let clean = Cleanup::new();
    clean.meta(&meta);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = Config::new(data_dir());
    config.upstream_base = upstream.base_url.clone();
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = format!("http://127.0.0.1:{port}");
    tweak(&mut config);
    let state = Arc::new(AppState::new(store, worlds, config));
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    let app = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fx { state, base: format!("http://127.0.0.1:{port}"), http: reqwest::Client::new(), clean }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self.http.post(format!("{}{path}", self.base)).json(&body).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    async fn delete(&self, path: &str) -> (u16, Value) {
        let r = self.http.delete(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    async fn wait(&self, branch: &str) -> Value {
        for _ in 0..600 {
            let (_, b) = self.get(&format!("/api/branches/{branch}")).await;
            if b["status"] != "running" {
                return b;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("branch still running");
    }
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

const ORDER: &str = "#W7181492";

fn gold_write() -> Value {
    let tasks = Tasks::load(&data_dir()).unwrap();
    let t = tasks.get("27").unwrap();
    let a = t["evaluation_criteria"]["actions"].as_array().unwrap().iter().find(|a| a["name"] == "exchange_delivered_order_items").unwrap();
    tool_msg(vec![("w1", "exchange_delivered_order_items", a["arguments"].clone())])
}

fn scripted_run() -> Vec<Value> {
    vec![
        tool_msg(vec![("t1", "find_user_id_by_name_zip", json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}))]),
        tool_msg(vec![("t2", "get_order_details", json!({"order_id": ORDER}))]),
        tool_msg(vec![("t3", "handoff", json!({"note": "Exchange an item in #W7181492."}))]),
        tool_msg(vec![("r1", "get_order_details", json!({"order_id": ORDER}))]),
        gold_write(),
        text_msg("Exchanged."),
    ]
}

async fn db_exists(fx: &Fx, name: &str) -> bool {
    fx.state.worlds.admin().exists(name).await.unwrap()
}

#[test]
fn delete_run_drops_its_databases() {
    run(async {
        let up = FakeUpstream::start(scripted_run()).await;
        let fx = setup_with("delete", &up, |_| {}).await;
        let (st, v) = fx.post("/api/runs", json!({"task_id": "27", "team": team()})).await;
        assert_eq!(st, 200, "{v}");
        let (run_id, b) = (v["run_id"].as_str().unwrap().to_string(), v["branch_id"].as_str().unwrap().to_string());
        assert_eq!(fx.wait(&b).await["status"], "passed");
        let snaps: Vec<String> = fx
            .state
            .store
            .events(&b)
            .await
            .unwrap()
            .iter()
            .filter_map(|e| e.payload["snapshot"].as_str().map(str::to_string))
            .collect();
        assert_eq!(snaps.len(), 1, "one successful write, one snapshot");
        assert!(db_exists(&fx, &format!("branch_{b}")).await && db_exists(&fx, &snaps[0]).await);

        let (st, v) = fx.delete(&format!("/api/runs/{run_id}")).await;
        assert_eq!((st, v["dropped"].as_i64()), (200, Some(2)), "{v}");
        assert!(!db_exists(&fx, &format!("branch_{b}")).await, "branch database is gone");
        assert!(!db_exists(&fx, &snaps[0]).await, "snapshot is gone");
        assert!(fx.state.store.runs().await.unwrap().is_empty());
        assert_eq!(fx.get(&format!("/api/branches/{b}")).await.0, 404);
        assert_eq!(fx.delete(&format!("/api/runs/{run_id}")).await.0, 404);
    })
}

#[test]
fn delete_running_run_is_409() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("running", &up, |_| {}).await;
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let b = fx.state.store.create_branch(&run_id, None, &t, &json!([]), None).await.unwrap();
        let (st, v) = fx.delete(&format!("/api/runs/{run_id}")).await;
        assert_eq!(st, 409, "{v}");
        assert!(fx.state.store.find_branch(&b).await.unwrap().is_some(), "nothing was deleted");
    })
}

#[test]
fn limit_refuses_new_runs() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("limit", &up, |c| c.max_db_bytes = 1).await;
        let (_, h) = fx.get("/api/health").await;
        assert_eq!(h["disk"]["limit_bytes"], 1);
        assert!(h["disk"]["bytes"].as_i64().unwrap() > 1 && h["disk"]["databases"].as_i64().unwrap() >= 1, "{h}");
        let (st, v) = fx.post("/api/runs", json!({"task_id": "27", "team": team()})).await;
        assert_eq!((st, v["error"].as_str()), (507, Some("the arena's databases use more than the limit; delete old runs")));
        let (st, _) = fx.post("/api/branches/whatever/fork", json!({"at_seq": 1})).await;
        assert_eq!(st, 507);
        assert!(fx.state.store.runs().await.unwrap().is_empty(), "no run was created");
    })
}

#[test]
fn startup_gc_drops_only_orphans() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("gc", &up, |_| {}).await;
        let (store, worlds) = (&fx.state.store, &fx.state.worlds);
        let t = team();
        let run_id = store.create_run("27", &t).await.unwrap();
        let b = store.create_branch(&run_id, None, &t, &json!([]), None).await.unwrap();
        let kept_branch = worlds.new_branch(&b, Source::Base).await.unwrap();
        let kept_snap = worlds.snapshot(&kept_branch).await.unwrap();
        let orphan_snap = worlds.snapshot(&kept_branch).await.unwrap();
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let orphan_branch = worlds.new_branch(&format!("gcorphan_{tag}"), Source::Base).await.unwrap();
        let temp_rows = worlds.new_branch(&format!("rows_{tag}"), Source::Base).await.unwrap();
        let other = format!("gcother_{tag}");
        worlds.admin().create_db_from(&other, "world_base").await.unwrap();
        for d in [&kept_snap, &orphan_snap, &orphan_branch, &temp_rows] {
            fx.clean.add(d);
        }
        fx.clean.add(&other);
        store
            .append(&b, Some("resolver"), EventKind::ToolCall, json!({"name": "x", "snapshot": kept_snap}))
            .await
            .unwrap();

        // Only this test's databases are candidates: the server also holds real runs.
        let all = vec![kept_branch.clone(), kept_snap.clone(), orphan_snap.clone(), orphan_branch.clone(), temp_rows.clone(), other.clone()];
        let mut cands = vec![];
        for d in &all[..5] {
            cands.push((d.clone(), 1_000_000));
        }
        let (n, bytes) = arenad::gc::collect_orphans_among(store, worlds, cands).await.unwrap();
        assert_eq!((n, bytes), (3, 3_000_000));
        for (d, want) in [(&kept_branch, true), (&kept_snap, true), (&orphan_snap, false), (&orphan_branch, false), (&temp_rows, false), (&other, true)] {
            assert_eq!(db_exists(&fx, d).await, want, "{d}");
        }
    })
}
