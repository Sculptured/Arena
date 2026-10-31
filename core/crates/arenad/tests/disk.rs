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
use world::{
    schema::load_base,
    snapshot::Worlds,
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

struct Fx {
    state: Arc<AppState>,
    base: String,
    http: reqwest::Client,
    clean: Cleanup,
}

async fn setup_with(tag: &str, upstream: &FakeUpstream, tweak: impl FnOnce(&mut Config)) -> Fx {
    let worlds = worlds().await;
    let meta = format!(
        "arena_test_disk_{}_{}",
        tag,
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    );
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
    Fx {
        state,
        base: format!("http://127.0.0.1:{port}"),
        http: reqwest::Client::new(),
        clean,
    }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self
            .http
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self
            .http
            .post(format!("{}{path}", self.base))
            .json(&body)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }
    async fn delete(&self, path: &str) -> (u16, Value) {
        let r = self
            .http
            .delete(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap();
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
    let a = t["evaluation_criteria"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "exchange_delivered_order_items")
        .unwrap();
    tool_msg(vec![(
        "w1",
        "exchange_delivered_order_items",
        a["arguments"].clone(),
    )])
}

fn scripted_run() -> Vec<Value> {
    vec![
        tool_msg(vec![(
            "t1",
            "find_user_id_by_name_zip",
            json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}),
        )]),
        tool_msg(vec![(
            "t2",
            "get_order_details",
            json!({"order_id": ORDER}),
        )]),
        tool_msg(vec![(
            "t3",
            "handoff",
            json!({"note": "Exchange an item in #W7181492."}),
        )]),
        tool_msg(vec![(
            "r1",
            "get_order_details",
            json!({"order_id": ORDER}),
        )]),
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
        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_eq!(st, 200, "{v}");
        let (run_id, b) = (
            v["run_id"].as_str().unwrap().to_string(),
            v["branch_id"].as_str().unwrap().to_string(),
        );
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
        assert!(
            !db_exists(&fx, &format!("branch_{b}")).await,
            "branch database is gone"
        );
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
        let b = fx
            .state
            .store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
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
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("limit", &up, |c| c.max_db_bytes = 1).await;
        let (_, h) = fx.get("/api/health").await;
        assert_eq!(h["disk"]["limit_bytes"], 1);
        assert!(
            h["disk"]["bytes"].as_i64().unwrap() > 1
                && h["disk"]["databases"].as_i64().unwrap() >= 1,
            "{h}"
        );
        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_eq!(
            (st, v["error"].as_str()),
            (
                507,
                Some("the arena's databases use more than the limit; delete old runs")
            )
        );
        // 404 and 409 are reported before 507.
        assert_eq!(
            fx.post("/api/branches/whatever/fork", json!({"at_seq": 1}))
                .await
                .0,
            404
        );
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let b = fx
            .state
            .store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
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
        assert_eq!(
            (st, v["error"].as_str()),
            (
                507,
                Some("the arena's databases use more than the limit; delete old runs")
            )
        );
        assert_eq!(
            fx.state.store.runs().await.unwrap()[0].branches.len(),
            1,
            "no branch was created"
        );
    })
}

#[test]
fn startup_gc_drops_only_orphans() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("gc", &up, |_| {}).await;
        let (store, worlds) = (&fx.state.store, &fx.state.worlds);
        // What the store treats as referenced: its branches' databases and recorded snapshots.
        let t = team();
        let run_id = store.create_run("27", &t).await.unwrap();
        let b = store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
        store
            .append(
                &b,
                Some("resolver"),
                EventKind::ToolCall,
                json!({"name": "x", "snapshot": "snap_abc"}),
            )
            .await
            .unwrap();
        store
            .append(
                &b,
                None,
                EventKind::RunStart,
                json!({"snapshot": "snap_not_a_tool_call"}),
            )
            .await
            .unwrap();
        let refs = store.referenced_dbs().await.unwrap();
        assert!(refs.contains(&format!("branch_{b}")) && refs.contains("snap_abc"));
        assert!(!refs.contains("snap_not_a_tool_call"));

        // The real selection path (`list_prefix` + exact prefixes) on unique test prefixes, so
        // the server's real databases can never be selected.
        let p = format!("gctest{}_", std::process::id());
        let (pb, ps) = (format!("{p}branch_"), format!("{p}snap_"));
        let name = |s: &str| format!("{p}{s}");
        let want_dropped = [format!("{pb}orphan"), format!("{ps}orphan")];
        let want_kept = [
            format!("{pb}keep"),    // referenced
            format!("{ps}keep"),    // referenced
            name("branchXorphan"),  // `_` is not a wildcard: not an exact prefix
            name("branchy_orphan"), // look-alike of `branch_`
            name("xbranch_orphan"), // prefix not at the start of the name part
            name("snap"),           // no trailing underscore
            name("gold_orphan"),    // other prefix
        ];
        for d in want_dropped.iter().chain(&want_kept) {
            worlds.admin().create_db_from(d, "template1").await.unwrap();
            fx.clean.add(d);
        }
        let keep: std::collections::HashSet<String> =
            [format!("{pb}keep"), format!("{ps}keep")].into();
        let (n, bytes) =
            arenad::gc::collect_orphans_with(worlds, &[pb.as_str(), ps.as_str()], &keep)
                .await
                .unwrap();
        assert_eq!(n, 2);
        assert!(bytes > 0);
        for d in &want_dropped {
            assert!(!db_exists(&fx, d).await, "{d} should be dropped");
        }
        for d in &want_kept {
            assert!(db_exists(&fx, d).await, "{d} should survive");
        }
    })
}
