//! Test suites: one team over a benchmark's tasks, each repeated, on the same seeded world and
//! the same checks. A built-in suite runs scripted agents one run after another; an external one
//! is claimed and driven run by run over HTTP, as a design in its own code would. The toy world
//! has two tasks (poke once, poke twice), a goal check and a safety check (never poke more than
//! asked). No model: the agents are scripted.
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
    pin::Pin,
    sync::{Arc, LazyLock, Weak},
    time::Duration,
};

use app_support::send;
use arenad::{
    agents::{AgentResult, AgentRun, Driver},
    app::{AppState, Config},
    exec::ToolExec,
    model::*,
    providers::ProviderConfig,
    store::{NewSuite, Store},
};
use fake_upstream::FakeUpstream;
use packages::{toy_world, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

const SYSTEM: &str =
    r#"{"kind": "erp", "product": "Toy ERP", "stands_in_for": "SAP S/4HANA", "simulated": true}"#;

/// Peeks once, then pokes as many times as its prompt says (`pokes:N`).
struct Poker(Weak<AppState>);

impl Poker {
    async fn act(&self, run: &AgentRun) -> Result<(), String> {
        let state = self.0.upgrade().ok_or("no app")?;
        let exec = ToolExec::new(state.store.clone());
        let n: usize = run
            .spec
            .prompt
            .trim_start_matches("pokes:")
            .parse()
            .unwrap();
        let (b, a) = (&run.branch, run.spec.id.as_str());
        exec.run_outcome(b, a, "peek", json!({}), None).await?;
        for _ in 0..n {
            exec.run_outcome(b, a, "poke", json!({}), None).await?;
        }
        Ok(())
    }
}

impl Driver for Poker {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let done = self.act(&run).await;
            AgentResult {
                ok: done.is_ok(),
                error: done.err(),
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
    base: String,
    http: reqwest::Client,
    _state: Arc<AppState>,
    _up: FakeUpstream,
    _hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

async fn setup(tag: &str) -> Fx {
    let bench = ["--scored", "--two-tasks", "--safety"];
    let root = toy_world(tag, &["--system", SYSTEM], &bench);
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("suites", tag).await;
    let (listener, base) = app_support::bind().await;
    let up = FakeUpstream::start(vec![]).await;
    let mut config = Config::new();
    config.providers = ProviderConfig::with_upstream(&up.base_url);
    config.self_base = base.clone();
    let state = AppState::new(store, hub.hub.clone(), config).with_benchmark("toybench");
    let state = Arc::new(state);
    let d: Arc<dyn Driver> = Arc::new(Poker(Arc::downgrade(&state)));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    app_support::serve(listener, &state);
    Fx {
        base,
        http: reqwest::Client::new(),
        _state: state,
        _up: up,
        _hub: hub,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        send(self.http.post(format!("{}{path}", self.base)).json(&body)).await
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        send(self.http.get(format!("{}{path}", self.base))).await
    }

    /// Creates a suite: its id.
    async fn suite(&self, body: Value) -> String {
        let (status, v) = self.post("/api/suites", body).await;
        assert_eq!(status, 200, "{v}");
        v["id"].as_str().unwrap().to_string()
    }

    /// The suite's report once it is finished.
    async fn finished(&self, id: &str) -> Value {
        for _ in 0..600 {
            let (status, v) = self.get(&format!("/api/suites/{id}")).await;
            assert_eq!(status, 200, "{v}");
            if v["status"] != "running" {
                assert_eq!(v["status"], "finished", "{v}");
                return v;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the suite did not finish");
    }
}

fn poker(pokes: usize) -> Value {
    let agent = json!({"id": "poker", "name": "Poker", "kind": "open", "model": "fake/model",
                       "prompt": format!("pokes:{pokes}")});
    json!({"id": format!("p{pokes}"), "name": format!("Pokes {pokes}"), "agents": [agent]})
}

fn builtin(pokes: usize) -> Value {
    json!({"team": poker(pokes), "tasks": ["t1", "t2"], "repeats": 2})
}

/// `[(task, check, passed, runs)]` of a report.
fn rates(r: &Value) -> Vec<(String, String, i64, i64)> {
    let row = |c: &Value| {
        let s = |k: &str| c[k].as_str().unwrap().to_string();
        (
            s("task_id"),
            s("name"),
            c["passed"].as_i64().unwrap(),
            c["runs"].as_i64().unwrap(),
        )
    };
    r["checks"].as_array().unwrap().iter().map(row).collect()
}

fn rate(task: &str, check: &str, passed: i64, runs: i64) -> (String, String, i64, i64) {
    (task.into(), check.into(), passed, runs)
}

#[test]
fn a_builtin_suite_of_two_tasks_by_two_repeats_aggregates_its_runs() {
    run(async {
        let fx = setup("builtin").await;
        let id = fx.suite(builtin(1)).await;
        let r = fx.finished(&id).await;
        assert_eq!(r["runs"].as_array().unwrap().len(), 4, "{r}");
        assert_eq!(
            rates(&r),
            vec![
                rate("t1", "poked", 2, 2),
                rate("t1", "not_overpoked", 2, 2),
                rate("t2", "poked", 0, 2),
                rate("t2", "not_overpoked", 2, 2),
            ]
        );
        let s = &r["summary"];
        assert_eq!(
            (&s["scored"], &s["passed"], &s["pass_rate"]),
            (&json!(4), &json!(2), &json!(0.5))
        );
        assert_eq!(
            (&s["safety_violations"], &s["score"]),
            (&json!(0), &json!(0.5))
        );
        assert_eq!(r["spread"][0]["checks_passed"]["mean"], 1.0);
        assert!(r["finished_s"].as_f64().unwrap() > 0.0, "{r}");
        assert_run_start_names_its_systems(&fx, &r).await;
    });
}

/// A run's `RunStart` says what kind of system the part is and which tools read or write it, and
/// each tool call how long it ran.
async fn assert_run_start_names_its_systems(fx: &Fx, r: &Value) {
    let branch = r["runs"][0]["branch_id"].as_str().unwrap();
    let (_, b) = fx.get(&format!("/api/branches/{branch}")).await;
    let start = &b["events"][0]["payload"];
    assert_eq!(
        start["systems"]["box"],
        serde_json::from_str::<Value>(SYSTEM).unwrap()
    );
    assert_eq!(
        start["tools"]["poke"],
        json!({"effect": "write", "part": "box"})
    );
    assert_eq!(
        start["tools"]["peek"],
        json!({"effect": "read", "part": "box"})
    );
    let events = b["events"].as_array().unwrap();
    let poke = events
        .iter()
        .find(|e| e["payload"]["name"] == "poke")
        .unwrap();
    assert!(
        poke["payload"]["duration_ms"].as_f64().unwrap() >= 0.0,
        "{poke}"
    );
    let (_, world) = fx.get("/api/world").await;
    assert_eq!(world["parts"][0]["system"]["product"], "Toy ERP");
}

/// The verdict on one task's check in a comparison.
fn verdict(c: &Value, task: &str, name: &str) -> String {
    let mut d = c["checks"].as_array().unwrap().iter();
    let d = d
        .find(|d| d["task_id"] == task && d["name"] == name)
        .unwrap();
    d["verdict"].as_str().unwrap().to_string()
}

#[test]
fn comparing_two_suites_flags_the_regression_and_counts_safety_apart() {
    run(async {
        let fx = setup("compare").await;
        let a = fx.suite(builtin(1)).await;
        let b = fx.suite(builtin(2)).await;
        fx.finished(&a).await;
        let rb = fx.finished(&b).await;
        // Poking twice meets t2's goal but trips t1's safety check: unsafe runs still score.
        assert_eq!(
            (
                &rb["summary"]["safety_violations"],
                &rb["summary"]["runs_with_violation"]
            ),
            (&json!(2), &json!(2))
        );
        assert_eq!(rb["summary"]["score"], 1.0);
        let (status, c) = fx.get(&format!("/api/suites/compare?a={a}&b={b}")).await;
        assert_eq!(status, 200, "{c}");
        assert_eq!(verdict(&c, "t1", "not_overpoked"), "regressed");
        assert_eq!(verdict(&c, "t2", "poked"), "improved");
        assert_eq!(verdict(&c, "t1", "poked"), "same");
        assert_eq!(
            (&c["safety"]["verdict"], &c["verdict"]),
            (&json!("regressed"), &json!("regressed"))
        );
    });
}

/// Claims the next run of an external suite: the claim, or None when it is done.
async fn claim(fx: &Fx, id: &str) -> Option<Value> {
    let (status, v) = fx.post(&format!("/api/suites/{id}/claim"), json!({})).await;
    assert_eq!(status, 200, "{v}");
    (v["done"] == false).then_some(v)
}

/// Drives one claimed run as an external system: an agent pokes once through its MCP endpoint,
/// its client reports a model call (more context each repeat), and the run is finished.
async fn drive(fx: &Fx, c: &Value) {
    let branch = c["branch_id"].as_str().unwrap();
    let (status, agent) = fx
        .post(
            &format!("/api/branches/{branch}/agents"),
            json!({"id": "ext"}),
        )
        .await;
    assert_eq!(status, 200, "{agent}");
    let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": "poke", "arguments": {}}});
    let (status, out) = send(fx.http.post(agent["mcp_url"].as_str().unwrap()).json(&call)).await;
    assert_eq!(
        (status, &out["result"]["isError"]),
        (200, &json!(false)),
        "{out}"
    );
    let input = 1000 * c["repeat"].as_i64().unwrap();
    let usage = json!({"message_id": "m", "input_tokens": input, "output_tokens": 10});
    let (status, v) = fx
        .post(&format!("/api/branches/{branch}/agents/ext/usage"), usage)
        .await;
    assert_eq!(status, 200, "{v}");
    let (status, v) = fx
        .post(&format!("/api/branches/{branch}/finish"), json!({}))
        .await;
    assert_eq!(status, 200, "{v}");
}

#[test]
fn an_external_system_claims_and_drives_each_run_of_a_suite() {
    run(async {
        let fx = setup("external").await;
        let body = json!({"team": {"id": "own", "name": "Own", "agents": []}, "tasks": ["t1"],
                          "repeats": 2, "runner": "external"});
        let id = fx.suite(body).await;
        let mut claimed = vec![];
        while let Some(c) = claim(&fx, &id).await {
            assert_eq!(c["task_id"], "t1");
            drive(&fx, &c).await;
            claimed.push(c["repeat"].as_i64().unwrap());
        }
        assert_eq!(claimed, vec![1, 2]);
        let r = fx.finished(&id).await;
        assert_eq!(
            (&r["summary"]["passed"], &r["runner"]),
            (&json!(2), &json!("external"))
        );
        let peak = &r["spread"][0]["peak_context"];
        assert_eq!(
            (&peak["min"], &peak["max"], &peak["mean"]),
            (&json!(1000.0), &json!(2000.0), &json!(1500.0))
        );
        assert_eq!(r["spread"][0]["tokens"]["max"], 2010.0);
    });
}

#[test]
fn suites_refuse_what_they_cannot_run() {
    run(async {
        let fx = setup("refuse").await;
        let bad = |body: Value| async { fx.post("/api/suites", body).await };
        let (status, v) = bad(json!({"team": poker(1), "repeats": 0})).await;
        assert_eq!(status, 400, "{v}");
        let (status, v) = bad(json!({"team": poker(1), "tasks": ["t9"]})).await;
        assert!(
            status == 400 && v["error"].as_str().unwrap().contains("t9"),
            "{v}"
        );
        let (status, _) = bad(json!({"team": poker(1), "tasks": ["t1", "t1"]})).await;
        assert_eq!(status, 400);
        let (status, _) = fx.get("/api/suites/nope").await;
        assert_eq!(status, 404);
        let id = fx.suite(json!({"team": poker(1), "tasks": ["t1"]})).await;
        let (status, v) = fx.post(&format!("/api/suites/{id}/claim"), json!({})).await;
        assert_eq!(status, 409, "{v}");
        fx.finished(&id).await;
        let (_, list) = fx.get("/api/suites").await;
        assert_eq!(
            (&list[0]["id"], &list[0]["status"], &list[0]["passed"]),
            (&json!(id), &json!("finished"), &json!(1))
        );
    });
}

/// A fresh metadata database's name, dropped with the returned guard.
fn fresh_db() -> (String, Cleanup) {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let name = format!("arena_test_suites_restart_{}", &id[..8]);
    let clean = Cleanup::new();
    clean.add(&name);
    (name, clean)
}

#[test]
fn a_builtin_suite_left_running_is_stopped_when_arenad_starts_again() {
    run(async {
        let (name, _clean) = fresh_db();
        let store = Store::open_named(&pg::admin(), &name).await.unwrap();
        let team: Team = serde_json::from_value(poker(1)).unwrap();
        let tasks = vec!["t1".to_string()];
        let scenario = Default::default();
        let new = |runner| NewSuite {
            name: "s",
            team: &team,
            tasks: &tasks,
            repeats: 1,
            runner,
            benchmark: "b",
            scenario: &scenario,
        };
        let builtin = store.create_suite(new(Runner::Chain)).await.unwrap();
        let external = store.create_suite(new(Runner::External)).await.unwrap();
        drop(store);
        let store = Store::open_named(&pg::admin(), &name).await.unwrap();
        let again = store.suite(&builtin.id).await.unwrap().unwrap();
        assert!(again.error.unwrap().contains("arenad stopped") && again.ended_at.is_some());
        let ext = store.suite(&external.id).await.unwrap().unwrap();
        assert_eq!(
            (ext.error, ext.ended_at),
            (None, None),
            "an external suite waits for its claims"
        );
    });
}

/// A suite keeps the expanded scenario its runs start in.
#[test]
fn a_suite_keeps_its_scenario() {
    run(async {
        let (name, _clean) = fresh_db();
        let store = Store::open_named(&pg::admin(), &name).await.unwrap();
        let team: Team = serde_json::from_value(poker(1)).unwrap();
        let tasks = vec!["t1".to_string()];
        let one = json!({"id": "jam", "label": "Jam", "part": "box", "params": {"after": 2},
                         "disruption": {"id": "jam-2"}});
        let scenario = serde_json::from_value(json!({"disruptions": [one]})).unwrap();
        let new = NewSuite {
            name: "s",
            team: &team,
            tasks: &tasks,
            repeats: 1,
            runner: Runner::External,
            benchmark: "b",
            scenario: &scenario,
        };
        let id = store.create_suite(new).await.unwrap().id;
        let again = store.suite(&id).await.unwrap().unwrap();
        assert_eq!(again.scenario, scenario);
        assert_eq!(again.scenario.disruptions[0].params["after"], 2);
    });
}
