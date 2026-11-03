//! `/api/world`, `/api/usage`, `/api/gc` and `/api/team/default` over toy integrations and a toy
//! benchmark.
#[path = "support/app.rs"]
mod app_support;
#[path = "support/fake_upstream.rs"]
mod fake_upstream;
#[path = "support/packages.rs"]
mod packages;
#[path = "support/pg.rs"]
mod pg;

use std::{
    future::Future,
    sync::{Arc, LazyLock},
};

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{AppState, Config},
    model::*,
    providers::ProviderConfig,
};
use fake_upstream::FakeUpstream;
use packages::{Package, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

/// An app whose benchmark is the toy `toybench`; its hub, metadata database and packages are
/// dropped with it.
struct Fx {
    state: Arc<AppState>,
    hub: TestHub,
    base: String,
    _root: ToyRoot,
    _clean: Cleanup,
}

/// A toy integration `name` started with `args`.
fn integration(name: &str, args: &[&str]) -> Package {
    let mut argv = vec!["part"];
    argv.extend_from_slice(args);
    packages::toy(name, "integration", &argv)
}

/// An app over the toy `integrations` and a benchmark whose parts are `parts` (`part=integration`).
async fn setup(tag: &str, integrations: Vec<Package>, parts: &[&str]) -> Fx {
    setup_with(tag, integrations, parts, None).await
}

/// `setup`, with open agents talking to the (fake) model upstream at `upstream`, if given.
async fn setup_with(
    tag: &str,
    integrations: Vec<Package>,
    parts: &[&str],
    upstream: Option<&str>,
) -> Fx {
    let mut bench = vec!["bench"];
    for p in parts {
        bench.extend(["--part", p]);
    }
    let mut all = integrations;
    all.push(packages::toy("toybench", "benchmark", &bench));
    let root = packages::package_root(tag, &all);
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("world", tag).await;
    let (listener, base) = app_support::bind().await;
    let mut config = Config::new();
    if let Some(url) = upstream {
        config.providers = ProviderConfig::with_upstream(url);
        config.self_base = base.clone();
    }
    let state = AppState::new(store, hub.hub.clone(), config).with_benchmark("toybench");
    let state = Arc::new(state);
    let driver: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, driver)].into_iter().collect()));
    app_support::serve(listener, &state);
    Fx {
        state,
        hub,
        base,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        let client = reqwest::Client::new();
        app_support::send(client.get(format!("{}{path}", self.base))).await
    }

    async fn post(&self, path: &str) -> (u16, Value) {
        let client = reqwest::Client::new();
        app_support::send(client.post(format!("{}{path}", self.base))).await
    }

    /// A toy save `save` in `integration`, with its folder.
    async fn save(&self, integration: &str, save: &str) {
        let c = self.state.hub.client(integration).await.unwrap();
        let start = json!({"instance": "mk", "config": {}, "seed": 0, "clock_ms": 0});
        c.call::<Value>("start", start).await.unwrap();
        let dir = self.state.hub.save_dir(integration, save);
        let params = json!({"instance": "mk", "save": save, "dir": dir});
        c.call::<Value>("save", params).await.unwrap();
    }

    async fn save_exists(&self, integration: &str, save: &str) -> bool {
        let params = json!({"target": {"save": save}, "keys": ["box/n"]});
        let c = self.state.hub.client(integration).await.unwrap();
        c.call::<Value>("inspect", params).await.is_ok()
    }

    /// A finished run of part `box` (of `integration`) whose only tool call saved `save`.
    async fn record_run(&self, integration: &str, save: &str) {
        let store = &self.state.store;
        let team = Team {
            id: "t".into(),
            name: "T".into(),
            agents: vec![],
        };
        let run_id = store.create_run("t1", &team).await.unwrap();
        let b = store
            .create_branch(&run_id, None, &team, &json!([]), None)
            .await
            .unwrap();
        let world = json!([{"part": "box", "integration": integration, "config": {}}]);
        let start = json!({"benchmark": "toybench", "world": world, "seed": 0, "saves": {}});
        store
            .append(&b, None, EventKind::RunStart, start)
            .await
            .unwrap();
        let call = json!({"name": "poke", "error": null, "writes": ["box/n"], "part": "box",
                          "saves": {"box": save}});
        store
            .append(&b, None, EventKind::ToolCall, call)
            .await
            .unwrap();
        let passed = store.set_status(&b, BranchStatus::Passed, None);
        passed.await.unwrap();
    }
}

/// The toy benchmark suggests no team: the default team is arenad's one generic agent.
#[test]
fn a_benchmark_without_a_team_gets_the_generic_one() {
    run(async {
        let fx = setup("noteam", vec![integration("toy-a", &[])], &["box=toy-a"]).await;
        let (st, v) = fx.get("/api/team/default").await;
        assert_eq!(st, 200, "{v}");
        let team: Team = serde_json::from_value(v).unwrap();
        assert_eq!(team.agents.len(), 1);
        assert_eq!(team.agents[0].prompt, arenad::prompts::GENERIC_PROMPT);
    })
}

#[test]
fn world_lists_parts_with_restore() {
    run(async {
        let part = integration("toy-a", &["--approximate", "--uses-clock"]);
        let fx = setup("worldok", vec![part], &["box=toy-a"]).await;
        let (st, v) = fx.get("/api/world").await;
        assert_eq!(st, 200, "{v}");
        let want = json!([{
            "name": "box", "integration": "toy-a", "answered": true, "version": "0.1.0",
            "restore": {"exact": "save", "approximate": true}, "uses_clock": true,
            "tools": ["poke", "peek"],
        }]);
        assert_eq!(v["parts"], want);
    })
}

#[test]
fn world_marks_unanswering_part() {
    run(async {
        let ok = integration("toy-ok", &[]);
        let bad = integration("toy-bad", &["--describe-fails"]);
        let parts = ["a=toy-ok", "b=toy-bad", "c=ghost"];
        let fx = setup("worldbad", vec![ok, bad], &parts).await;
        let (st, v) = fx.get("/api/world").await;
        assert_eq!(st, 200, "{v}");
        let answered: Vec<_> = v["parts"].as_array().unwrap().iter().collect();
        assert_eq!(answered[0]["answered"], true);
        assert_eq!(answered[1]["answered"], false);
        let error = answered[1]["error"].as_str().unwrap();
        assert!(error.contains("the toy cannot describe itself"), "{error}");
        assert_eq!(answered[2]["answered"], false);
        let error = answered[2]["error"].as_str().unwrap();
        assert!(error.contains("no package named ghost"), "{error}");
        assert!(answered[1].get("version").is_none(), "{v}");
    })
}

#[test]
fn usage_sums_parts() {
    run(async {
        let both = vec![integration("toy-a", &[]), integration("toy-b", &[])];
        let fx = setup("usage", both, &["x=toy-a", "y=toy-b", "z=toy-a"]).await;
        fx.save("toy-a", "s_1").await;
        fx.save("toy-a", "s_2").await;
        fx.save("toy-b", "s_3").await;
        let (st, v) = fx.get("/api/usage").await;
        assert_eq!(st, 200, "{v}");
        let want = json!([
            {"integration": "toy-a", "parts": ["x", "z"], "bytes": 2},
            {"integration": "toy-b", "parts": ["y"], "bytes": 1},
        ]);
        assert_eq!(v["integrations"], want);
        assert_eq!(v["total_bytes"], 3);
        assert_eq!(v["limit_bytes"], fx.state.config.max_db_bytes);
        let (_, health) = fx.get("/api/health").await;
        assert_eq!(health["disk"]["bytes"], v["total_bytes"]);
    })
}

#[test]
fn gc_keeps_referenced_saves() {
    run(async {
        let fx = setup("gc", vec![integration("toy-a", &["--gc"])], &["box=toy-a"]).await;
        fx.save("toy-a", "s_kept").await;
        fx.save("toy-a", "s_orphan").await;
        fx.record_run("toy-a", "s_kept").await;
        let folder = |s: &str| fx.hub.saves.join("toy-a").join(s);

        let (st, v) = fx.post("/api/gc").await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["removed"], json!(["s_orphan"]));
        assert_eq!(v["freed_bytes"], 1);
        assert!(!fx.save_exists("toy-a", "s_orphan").await);
        assert!(!folder("s_orphan").exists());
        assert!(fx.save_exists("toy-a", "s_kept").await);
        assert!(folder("s_kept").is_dir());
    })
}

#[test]
fn world_bounds_hung_part() {
    run(async {
        let ok = integration("toy-ok", &[]);
        let hung = integration("toy-hung", &["--describe-hangs"]);
        let fx = setup("worldhung", vec![ok, hung], &["a=toy-ok", "b=toy-hung"]).await;
        let started = std::time::Instant::now();
        let (st, v) = fx.get("/api/world").await;
        assert_eq!(st, 200, "{v}");
        assert!(started.elapsed().as_secs() < 15, "{:?}", started.elapsed());
        assert_eq!(v["parts"][0]["answered"], true);
        assert_eq!(v["parts"][1]["answered"], false);
        assert_eq!(v["parts"][1]["error"], "did not answer within 5 s");
        // The timed-out call left the shared client usable: its pending call is gone.
        let client = fx.state.hub.client("toy-hung").await.unwrap();
        assert_eq!(client.pending_len(), 0);
    })
}

/// A branch of the toy world, running as far as `state.live` is concerned.
async fn make_live(fx: &Fx) {
    let world = fx.hub.start_world("toybench", "b1").await;
    let team = Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![],
    };
    let ctx = arenad::exec::BranchCtx::new("b1".into(), world, team, "t1".into());
    fx.state
        .live
        .write()
        .unwrap()
        .insert("b1".into(), Arc::new(ctx));
}

#[test]
fn gc_refused_while_branch_live() {
    run(async {
        let fx = setup(
            "gclive",
            vec![integration("toy-a", &["--gc"])],
            &["box=toy-a"],
        )
        .await;
        fx.save("toy-a", "s_orphan").await;
        make_live(&fx).await;
        let (st, v) = fx.post("/api/gc").await;
        assert_eq!(st, 409, "{v}");
        assert_eq!(
            v["error"],
            "agents are running; clean up when they have stopped"
        );
        assert!(
            fx.save_exists("toy-a", "s_orphan").await,
            "nothing was dropped"
        );
        fx.state.live.write().unwrap().clear();
        assert_eq!(fx.post("/api/gc").await.0, 200);
    })
}

#[test]
fn run_refused_during_gc() {
    run(async {
        let fx = setup(
            "gclock",
            vec![integration("toy-a", &["--gc"])],
            &["box=toy-a"],
        )
        .await;
        let held = fx.state.cleanup.clone().write_owned().await;
        let want = "Saves are being cleaned up; try again in a moment";
        for path in ["/api/runs", "/api/branches/b/fork", "/api/gc"] {
            let (st, v) = fx.post(path).await;
            assert_eq!((st, v["error"].as_str()), (409, Some(want)), "{path}");
        }
        drop(held);
        assert_eq!(fx.post("/api/gc").await.0, 200);
    })
}

#[test]
fn gc_reports_failing_integration() {
    run(async {
        let part = integration("toy-a", &["--gc", "--gc-fails"]);
        let fx = setup("gcfails", vec![part], &["box=toy-a"]).await;
        let (st, v) = fx.post("/api/gc").await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["removed"], json!([]));
        let errors = v["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{v}");
        assert_eq!(errors[0]["integration"], "toy-a");
        let message = errors[0]["message"].as_str().unwrap();
        assert!(
            message.contains("the toy cannot clean up today"),
            "{message}"
        );
    })
}

/// A run that has just been created is live at once: a gc right after `POST /api/runs` returns
/// is refused, with no gap for it to drop the run's instances.
#[test]
fn gc_refused_right_after_run_starts() {
    run(async {
        let slow = json!({"role": "assistant", "content": "Done.", "_delay_ms": 3000});
        let up = FakeUpstream::start(vec![slow]).await;
        let part = integration("toy-a", &["--gc"]);
        let fx = setup_with("gcnow", vec![part], &["box=toy-a"], Some(&up.base_url)).await;
        let agent = json!({"id": "a", "name": "A", "kind": "open", "model": "fake/model",
                           "prompt": "You are A."});
        let team = json!({"id": "t", "name": "T", "agents": [agent]});
        let client = reqwest::Client::new();
        let req = client.post(format!("{}/api/runs", fx.base));
        let (st, v) = app_support::send(req.json(&json!({"task_id": "t1", "team": team}))).await;
        assert_eq!(st, 200, "{v}");
        let (st, v) = fx.post("/api/gc").await;
        assert_eq!(st, 409, "{v}");
        for _ in 0..100 {
            if fx.state.live.read().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
}
