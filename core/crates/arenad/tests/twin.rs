//! A run's scenario and the world's view. A scenario asked for on `POST /api/runs` is expanded
//! with the benchmark's templates into the task's setup and recorded in `RunStart`; the world's
//! view is recorded after the setup and after every step that wrote, a fork's views include those
//! before its fork point, and a view that fails records nothing and fails nothing.
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
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{AppState, Config},
    model::*,
    providers::ProviderConfig,
};
use fake_upstream::FakeUpstream;
use packages::{package_root, toy, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn solo() -> Value {
    json!({"id": "t", "name": "T", "agents": [{"id": "solo", "name": "solo", "kind": "open",
           "model": "fake/model", "prompt": "You are solo."}]})
}

fn call(name: &str) -> Value {
    let tc = json!({"id": "c", "type": "function", "function": {"name": name, "arguments": "{}"}});
    json!({"role": "assistant", "content": null, "tool_calls": [tc]})
}

fn done() -> Value {
    json!({"role": "assistant", "content": "Done."})
}

/// One disruption the builder may add: `jam` on part `a`, after `{after}` calls.
fn options() -> Value {
    let after =
        json!({"name": "after", "label": "After", "kind": "int", "min": 0, "max": 9, "default": 1});
    let jam = json!({"id": "jam", "label": "Jam", "description": "The box jams after {after} calls",
        "part": "a", "params": [after],
        "template": {"id": "jam-{after}", "when": {"after_calls": "{after}"}, "do": {"kind": "jam"},
                     "text": "The box jams"}});
    json!({"base": ["t1"], "disruptions": [jam], "setup": {"a": {"tool": "a_poke", "args": {}}}})
}

/// Parts `a` and `b`; the view reads `b`. `extra` goes to the benchmark's arguments.
fn world(tag: &str, extra: &[&str]) -> ToyRoot {
    world_with(tag, extra, &[])
}

/// `world`, with `part` given to `b`'s integration as well.
fn world_with(tag: &str, extra: &[&str], part: &[&str]) -> ToyRoot {
    let options = options().to_string();
    let mut args = vec![
        "bench", "--scored", "--part", "a=toy-a", "--part", "b=toy-b",
    ];
    args.extend(["--view", "b", "--scenario-options", options.as_str()]);
    args.extend(extra);
    package_root(
        tag,
        &[
            toy("toy-a", "integration", &["part", "--prefix", "a_"]),
            toy(
                "toy-b",
                "integration",
                &[&["part", "--prefix", "b_"], part].concat(),
            ),
            toy("toybench", "benchmark", &args),
        ],
    )
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    _hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

async fn setup(tag: &str, root: ToyRoot, up: &FakeUpstream) -> Fx {
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("twin", tag).await;
    let (listener, base) = app_support::bind().await;
    let mut config = Config::new();
    config.providers = ProviderConfig::with_upstream(&up.base_url);
    config.self_base = base.clone();
    let state = AppState::new(store, hub.hub.clone(), config).with_benchmark("toybench");
    let state = Arc::new(state);
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    app_support::serve(listener, &state);
    Fx {
        state,
        base,
        _hub: hub,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let url = format!("{}{path}", self.base);
        app_support::send(reqwest::Client::new().post(url).json(&body)).await
    }

    async fn get(&self, path: &str) -> Value {
        let url = format!("{}{path}", self.base);
        let (status, v) = app_support::send(reqwest::Client::new().get(url)).await;
        assert_eq!(status, 200, "{v}");
        v
    }

    async fn finished(&self, branch: &str) -> Vec<Event> {
        for _ in 0..600 {
            let b = self.state.store.branch(branch).await.unwrap();
            if b.status != BranchStatus::Running {
                assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
                return self.state.store.events(branch).await.unwrap();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("branch still running");
    }

    /// Forks `parent` at `at_seq` with no change; the new branch's events, run to its end.
    async fn fork_to_end(&self, parent: &str, at_seq: i64) -> Vec<Event> {
        let fork = json!({"at_seq": at_seq});
        let (status, v) = self
            .post(&format!("/api/branches/{parent}/fork"), fork)
            .await;
        assert_eq!(status, 200, "{v}");
        self.finished(v["branch_id"].as_str().unwrap()).await
    }

    /// Starts a run of `t1` with `scenario`; its branch, run to its end, and its events.
    async fn run_in(&self, scenario: Value) -> (String, Vec<Event>) {
        let body = json!({"task_id": "t1", "team": solo(), "scenario": scenario});
        let (status, v) = self.post("/api/runs", body).await;
        assert_eq!(status, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let events = self.finished(&branch).await;
        (branch, events)
    }
}

fn steps<'a>(evs: &'a [Event], actor: &str) -> Vec<&'a Event> {
    let of = |e: &&Event| e.kind == EventKind::ToolCall && e.agent.as_deref() == Some(actor);
    evs.iter().filter(of).collect()
}

fn model_call(evs: &[Event], n: usize) -> i64 {
    let calls = evs.iter().filter(|e| e.kind == EventKind::ModelCall);
    calls.map(|e| e.seq).nth(n).unwrap()
}

/// `(seq, n)` of each view: the step it follows and the item's status.
fn drawn(views: &Value) -> Vec<(i64, String)> {
    let one = |v: &Value| {
        let status = v["view"]["areas"][0]["items"][0]["status"]
            .as_str()
            .unwrap();
        (v["seq"].as_i64().unwrap(), status.to_string())
    };
    views.as_array().unwrap().iter().map(one).collect()
}

/// The chosen disruption is expanded into `a`'s setup call and recorded in `RunStart`, a bad
/// choice is refused, and the benchmark's options are offered.
#[test]
fn a_scenario_is_expanded_into_the_setup_and_recorded() {
    run(async {
        let up = FakeUpstream::start(vec![call("b_peek"), done()]).await;
        let fx = setup("scen", world("scen", &[]), &up).await;
        let offered = fx.get("/api/benchmark").await;
        assert_eq!(offered["scenario_options"]["disruptions"][0]["id"], "jam");
        assert_eq!(offered["view"], true);
        let bad = json!({"disruptions": [{"id": "jam", "params": {"after": 99}}]});
        let body = json!({"task_id": "t1", "team": solo(), "scenario": bad});
        let (status, v) = fx.post("/api/runs", body).await;
        assert_eq!(
            (
                status,
                v["error"].as_str().unwrap().contains("after must be from")
            ),
            (400, true),
            "{v}"
        );
        let (_, evs) = fx
            .run_in(json!({"disruptions": [{"id": "jam", "params": {"after": 2}}]}))
            .await;
        let start = &evs[0].payload["scenario"]["disruptions"][0];
        assert_eq!((&start["id"], &start["part"]), (&json!("jam"), &json!("a")));
        assert_eq!(start["disruption"]["when"]["after_calls"], 2, "{start}");
        let set = steps(&evs, "setup");
        assert_eq!(set.len(), 1, "a setup call added for the part");
        assert_eq!(set[0].payload["name"], "a_poke");
        assert_eq!(set[0].payload["args"]["disruptions"][0]["id"], "jam-2");
        let (_, plain) = fx.run_in(json!({"disruptions": []})).await;
        assert!(plain[0].payload.get("scenario").is_none() && steps(&plain, "setup").is_empty());
    })
}

/// A view after the setup and after each step that wrote (not after a read), and a fork has the
/// views before its fork point, then its own.
#[test]
fn views_follow_the_writing_steps_and_forks_inherit_them() {
    run(async {
        let script = vec![call("b_poke"), call("b_peek"), call("b_poke"), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("views", world("views", &[]), &up).await;
        let (parent, evs) = fx
            .run_in(json!({"disruptions": [{"id": "jam", "params": {}}]}))
            .await;
        let (set, pokes) = (steps(&evs, "setup"), steps(&evs, "solo"));
        let views = fx.get(&format!("/api/branches/{parent}/views")).await;
        let want = [(set[0].seq, "0"), (pokes[0].seq, "1"), (pokes[2].seq, "2")];
        let want: Vec<(i64, String)> = want.iter().map(|(s, n)| (*s, n.to_string())).collect();
        assert_eq!(drawn(&views), want);
        assert!(views[0]["event_seq"].as_i64().unwrap() > set[0].seq);
        let at = model_call(&evs, 2);
        let (status, v) = fx
            .post(
                &format!("/api/branches/{parent}/fork"),
                json!({"at_seq": at}),
            )
            .await;
        assert_eq!(status, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap();
        let cevs = fx.finished(child).await;
        assert!(
            cevs[0].payload["scenario"]["disruptions"][0]["id"] == "jam",
            "the fork keeps it"
        );
        let inherited = fx.get(&format!("/api/branches/{child}/views")).await;
        assert_eq!(drawn(&inherited)[..2], want[..2]);
    })
}

/// A run without a scenario still gets a view after its start; a view that fails records nothing
/// and the run goes on.
#[test]
fn a_failing_view_records_nothing_and_fails_nothing() {
    run(async {
        let up = FakeUpstream::start(vec![call("b_poke"), done()]).await;
        let fx = setup("noview", world("noview", &["--view-fails"]), &up).await;
        let (_, evs) = fx.run_in(Value::Null).await;
        assert!(evs.iter().all(|e| e.kind != EventKind::WorldView));
        assert_eq!(steps(&evs, "solo").len(), 1);
        let up = FakeUpstream::start(vec![done()]).await;
        let fx = setup("startview", world("startview", &[]), &up).await;
        let (branch, _) = fx.run_in(Value::Null).await;
        let views = fx.get(&format!("/api/branches/{branch}/views")).await;
        assert_eq!(drawn(&views), [(1, "0".to_string())], "after RunStart");
    })
}

/// A read that changes only its part's internal keys (`_peeks/n`) is saved, so a fork restores
/// them exactly, but it is recorded as no write and gets no view.
#[test]
fn internal_keys_are_saved_but_make_no_write_and_no_view() {
    run(async {
        let script = vec![call("b_peek"), call("b_peek"), call("b_poke"), done()];
        let up = FakeUpstream::start([script, vec![call("b_peek"), done()]].concat()).await;
        let root = world_with("internal", &[], &["--internal-peeks"]);
        let fx = setup("internal", root, &up).await;
        let (parent, evs) = fx.run_in(Value::Null).await;
        let solo = steps(&evs, "solo");
        let peek = &solo[1].payload;
        let keys = (&peek["writes"], &peek["internal"]);
        assert_eq!(keys, (&json!([]), &json!(["_peeks/n"])));
        assert!(
            peek["saves"]["b"].is_string(),
            "saved for its internal key: {peek}"
        );
        let views = fx.get(&format!("/api/branches/{parent}/views")).await;
        let seqs: Vec<i64> = drawn(&views).into_iter().map(|(s, _)| s).collect();
        assert_eq!(
            seqs,
            [1, solo[2].seq],
            "after RunStart and the poke, not the peeks"
        );
        let cevs = fx.fork_to_end(&parent, model_call(&evs, 2)).await;
        let again = steps(&cevs, "solo").into_iter().last().unwrap();
        let result = &again.payload["result"];
        assert_eq!(
            result["peeks"], 3,
            "the fork restored the two peeks counted: {result}"
        );
    })
}
