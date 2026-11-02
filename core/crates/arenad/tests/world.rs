//! `/api/world`, `/api/usage` and `/api/gc` over toy integrations and a toy benchmark.
#[path = "support/app.rs"]
mod app_support;
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
    let state = AppState::new(store, hub.hub.clone(), Config::new()).with_benchmark("toybench");
    let state = Arc::new(state);
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
        let fx = setup("usage", both, &["x=toy-a", "y=toy-b"]).await;
        fx.save("toy-a", "s_1").await;
        fx.save("toy-a", "s_2").await;
        fx.save("toy-b", "s_3").await;
        let (st, v) = fx.get("/api/usage").await;
        assert_eq!(st, 200, "{v}");
        let want = json!([{"name": "toy-a", "bytes": 2}, {"name": "toy-b", "bytes": 1}]);
        assert_eq!(v["parts"], want);
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
