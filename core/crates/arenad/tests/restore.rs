//! Rerunning worlds whose parts restore in different ways: exact and approximate saves, replay,
//! fingerprints, the arena clock and the restore plan. The worlds are made of the kit's `counter`
//! example (in its restore flavours) and toy parts, run by a scripted open agent.
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
    runner,
    store::Store,
};
use fake_upstream::FakeUpstream;
use packages::{counter, package_root, toy, Package, TestHub, ToyRoot};
use parts::Hub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn solo() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![AgentSpec {
            id: "solo".into(),
            name: "solo".into(),
            kind: AgentKind::Open,
            model: Some("fake/model".into()),
            prompt: "You are solo.".into(),
        }],
    }
}

fn call(name: &str, args: Value) -> Value {
    let tc = json!({"id": "c", "type": "function", "function": {"name": name, "arguments": args.to_string()}});
    json!({"role": "assistant", "content": null, "tool_calls": [tc]})
}

fn add(n: i64) -> Value {
    call("add", json!({"n": n}))
}

fn get() -> Value {
    call("get", json!({}))
}

fn done() -> Value {
    json!({"role": "assistant", "content": "Done."})
}

/// The toy benchmark over `parts` (`name=integration`), whose gold is `gold` with `gold_args`.
fn bench(parts: &[&str], gold: &str, gold_args: &str) -> Package {
    let mut args = vec!["bench", "--gold", gold, "--gold-args", gold_args];
    for p in parts {
        args.extend(["--part", p]);
    }
    toy("toybench", "benchmark", &args)
}

/// A world of one part `c`: the counter in `flavour` (plus `extra` args).
fn counter_world(tag: &str, flavour: &str, extra: &[&str]) -> ToyRoot {
    let mut args = vec!["--flavour", flavour];
    args.extend_from_slice(extra);
    package_root(
        tag,
        &[
            counter("counter", &args),
            bench(&["c=counter"], "add", r#"{"n": 1}"#),
        ],
    )
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

/// An app served on a free port whose benchmark is `toybench` on `hub`, its open agent talking
/// to `up`.
async fn serve_app(store: Arc<Store>, hub: Arc<Hub>, up: &FakeUpstream) -> (Arc<AppState>, String) {
    let (listener, base) = app_support::bind().await;
    let mut config = Config::new();
    config.upstream_base = up.base_url.clone();
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = base.clone();
    let state = Arc::new(AppState::new(store, hub, config).with_benchmark("toybench"));
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    app_support::serve(listener, &state);
    (state, base)
}

async fn setup(tag: &str, root: ToyRoot, up: &FakeUpstream) -> Fx {
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("restore", tag).await;
    let (state, base) = serve_app(store, hub.hub.clone(), up).await;
    Fx {
        state,
        base,
        hub,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(reqwest::Client::new().get(format!("{}{path}", self.base))).await
    }

    async fn fork(&self, branch: &str, at_seq: i64, restore: Option<&str>) -> (u16, Value) {
        fork_on(&self.base, branch, at_seq, restore).await
    }

    async fn finished(&self, branch: &str) -> Branch {
        for _ in 0..600 {
            let b = self.state.store.branch(branch).await.unwrap();
            if b.status != BranchStatus::Running {
                return b;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("branch still running");
    }

    /// Runs the solo agent on task `t1` to the end; the branch and its events.
    async fn run_to_end(&self) -> (String, Vec<Event>) {
        let (_run, branch) = runner::start_run(&self.state, "t1", solo()).await.unwrap();
        let b = self.finished(&branch).await;
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        let events = self.state.store.events(&branch).await.unwrap();
        (branch, events)
    }
}

async fn fork_on(base: &str, branch: &str, at_seq: i64, restore: Option<&str>) -> (u16, Value) {
    let mut body = json!({"at_seq": at_seq, "changes": {}});
    if let Some(r) = restore {
        body["restore"] = json!(r);
    }
    let url = format!("{base}/api/branches/{branch}/fork");
    app_support::send(reqwest::Client::new().post(url).json(&body)).await
}

/// The seq of the `n`-th (from 0) model call, the one whose reply makes the `n`-th tool call.
fn model_call(evs: &[Event], n: usize) -> i64 {
    evs.iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .nth(n)
        .unwrap()
        .seq
}

fn tool_calls(evs: &[Event]) -> Vec<&Event> {
    evs.iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .collect()
}

/// The `value` the branch's first `get` at or after `from` returned.
fn first_get(evs: &[Event], from: i64) -> i64 {
    tool_calls(evs)
        .into_iter()
        .find(|e| e.seq >= from && e.payload["name"] == "get")
        .and_then(|e| e.payload["result"]["value"].as_i64())
        .unwrap_or_else(|| panic!("no get after {from}"))
}

/// The child's state continued from the 1000 added before the fork point (the clock adds one
/// per second on top, so a little more is fine; a fresh counter would be near 0).
fn assert_continued(value: i64) {
    assert!((1000..1100).contains(&value), "{value}");
}

#[test]
fn exact_refused_names_part() {
    run(async {
        let root = package_root(
            "exactno",
            &[
                toy(
                    "toy-ax",
                    "integration",
                    &["part", "--exact", "none", "--approximate"],
                ),
                bench(&["box=toy-ax"], "poke", "{}"),
            ],
        );
        let poke = call("poke", json!({}));
        let up = FakeUpstream::start(vec![poke.clone(), poke, done()]).await;
        let fx = setup("exactno", root, &up).await;
        let (branch, evs) = fx.run_to_end().await;
        let at = model_call(&evs, 1);
        for restore in [None, Some("exact")] {
            let (st, v) = fx.fork(&branch, at, restore).await;
            assert_eq!(st, 409, "{v}");
            let msg = v["error"].as_str().unwrap();
            assert!(msg.contains("box") && msg.contains("approximate"), "{msg}");
        }
        // Refused before anything was made: the run still has only its first branch.
        let runs = fx.state.store.runs().await.unwrap();
        assert_eq!(runs[0].branches.len(), 1);
    })
}

/// Forks `parent` at `at` with `restore` (200), waits for the child to finish (not
/// `infra_error`) and checks it went on from the 1000 added before the fork point; its id.
async fn fork_and_finish(fx: &Fx, parent: &str, at: i64, restore: Option<&str>) -> String {
    let (st, v) = fx.fork(parent, at, restore).await;
    assert_eq!(st, 200, "{v}");
    let child = v["branch_id"].as_str().unwrap().to_string();
    let b = fx.finished(&child).await;
    assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
    assert_continued(first_get(&fx.state.store.events(&child).await.unwrap(), at));
    child
}

/// `/api/branches/{child}` shows its restore mode and how its one part `c` was restored.
async fn assert_restored(fx: &Fx, child: &str, mode: &str, how: &str) {
    let (_, v) = fx.get(&format!("/api/branches/{child}")).await;
    assert_eq!(v["restore_mode"], mode, "{v}");
    assert_eq!(v["restored"], json!([{"part": "c", "how": how}]));
}

/// The run list and compare show the child's restore mode, and none for the run's first branch.
async fn assert_listed(fx: &Fx, parent: &str, child: &str) {
    let (_, runs) = fx.get("/api/runs").await;
    let branches = runs[0]["branches"].as_array().unwrap();
    let of = |id: &str| branches.iter().find(|b| b["id"] == id).unwrap().clone();
    assert_eq!(of(parent)["restore_mode"], Value::Null);
    assert_eq!(of(parent)["restored"], json!([]));
    assert_eq!(of(child)["restore_mode"], "approximate");
    let (st, v) = fx.get(&format!("/api/compare?a={parent}&b={child}")).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["a"]["restore_mode"], Value::Null);
    assert_eq!(v["b"]["restore_mode"], "approximate");
}

#[test]
fn approximate_marks_branch() {
    run(async {
        let root = counter_world("approx", "approx_and_replay", &[]);
        let up = FakeUpstream::start(vec![add(1000), add(7), done(), get(), done()]).await;
        let fx = setup("approx", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let child = fork_and_finish(&fx, &parent, model_call(&evs, 1), Some("approximate")).await;
        assert_restored(&fx, &child, "approximate", "approximate").await;
        assert_listed(&fx, &parent, &child).await;
    })
}

#[test]
fn replay_only_restores_exactly() {
    run(async {
        let root = counter_world("replay", "replay_only", &[]);
        let script = vec![add(1000), get(), add(7), done(), get(), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("replay", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        // A part with only replay is never saved.
        assert!(tool_calls(&evs)
            .iter()
            .all(|e| e.payload["saves"].is_null()));
        let child = fork_and_finish(&fx, &parent, model_call(&evs, 2), None).await;
        assert_restored(&fx, &child, "exact", "replay").await;
    })
}

#[test]
fn replay_divergence_fails() {
    run(async {
        let root = counter_world("noisy", "nondeterministic", &[]);
        let up = FakeUpstream::start(vec![add(1), add(2), add(3), done()]).await;
        let fx = setup("noisy", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let first_add = tool_calls(&evs)[0].seq;
        let (st, v) = fx.fork(&parent, model_call(&evs, 2), None).await;
        assert_eq!(st, 409, "{v}");
        let want =
            format!("replay of c diverged at step {first_add}, so it can't be restored exactly");
        assert_eq!(v["error"], want.as_str());
    })
}

#[test]
fn fingerprint_mismatch_refused() {
    run(async {
        let root = counter_world("fpa", "replay_only", &["--fingerprint", "a"]);
        let other = counter_world("fpb", "replay_only", &["--fingerprint", "b"]);
        let up = FakeUpstream::start(vec![add(1), add(2), done()]).await;
        let fx = setup("fp", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        // The same store and saves, with the counter now on another machine.
        let hub = fx.hub.sibling(std::slice::from_ref(&other.0));
        let (_state, base) = serve_app(fx.state.store.clone(), hub, &up).await;
        let (st, v) = fork_on(&base, &parent, model_call(&evs, 1), None).await;
        assert_eq!(st, 409, "{v}");
        let want =
            "the c integration is running on a different machine or version than the original";
        assert_eq!(v["error"], want);
    })
}

/// The `advance` requests the toy part logged for `instance`.
fn advances(log: &std::path::Path, instance: &str) -> usize {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .filter(|l| *l == format!("advance {instance}"))
        .count()
}

/// A world of one toy clock part `box` whose saves take 400 ms and that logs its requests to
/// `log`.
fn clock_world(log: &std::path::Path) -> ToyRoot {
    let log = log.to_string_lossy();
    let part = ["part", "--uses-clock", "--save-sleep", "400", "--log", &log];
    package_root(
        "clock",
        &[
            toy("toy-clock", "integration", &part),
            bench(&["box=toy-clock"], "poke", "{}"),
        ],
    )
}

/// Each call saved the clock part (its advance reported a change) with the clock paused, so
/// the clock moved less than real time between them.
fn assert_paused_saves(poke: &Event, peek: &Event) {
    for c in [poke, peek] {
        assert!(c.payload["saves"]["box"].is_string(), "{}", c.payload);
        let paused = c.payload["paused_ms"].as_i64().unwrap();
        assert!(paused >= 350, "{}", c.payload);
    }
    let clock = |e: &Event| e.payload["clock_ms"].as_i64().unwrap();
    let real = (peek.at - poke.at).num_milliseconds();
    let moved = clock(peek) - clock(poke);
    assert!(real - moved >= 300, "real {real} ms, clock {moved} ms");
}

/// The part's event was recorded as a world event with the clock it happened at.
fn assert_world_event(evs: &[Event]) {
    let we = evs
        .iter()
        .find(|e| e.kind == EventKind::WorldEvent)
        .expect("a world event");
    assert_eq!(we.payload["part"], "box");
    assert_eq!(we.payload["kind"], "alarm");
    let at = we.payload["clock_ms"].as_i64();
    assert!(at.is_some_and(|c| c >= 0), "{}", we.payload);
}

/// The ticker advanced the part between calls too, and stopped with the branch.
async fn assert_ticked_then_stopped(log: &std::path::Path, branch: &str) {
    let instance = arenad::world::instance_id(branch, "box");
    let n = advances(log, &instance);
    assert!(n >= 4, "{n} advances");
    assert!(!arenad::ticker::is_running(branch));
    tokio::time::sleep(Duration::from_millis(600)).await;
    let after = advances(log, &instance);
    assert_eq!(after, n, "no tick after the branch ended");
}

#[test]
fn clock_advances_and_pauses_during_save() {
    run(async {
        let logs = ToyRoot(packages::toy_dir("clocklog"));
        std::fs::create_dir_all(&logs.0).unwrap();
        let log = logs.0.join("calls.log");
        let script = vec![call("poke", json!({})), call("peek", json!({})), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("clock", clock_world(&log), &up).await;
        let (branch, evs) = fx.run_to_end().await;
        let calls = tool_calls(&evs);
        assert_paused_saves(calls[0], calls[1]);
        assert_world_event(&evs);
        assert_ticked_then_stopped(&log, &branch).await;
    })
}

/// A world of three parts: `c` replay only, `box` exact saves, `ax` approximate saves only.
fn plan_world() -> ToyRoot {
    let approx_only = [
        "part",
        "--exact",
        "none",
        "--approximate",
        "--prefix",
        "ax_",
    ];
    package_root(
        "plan",
        &[
            counter("counter", &["--flavour", "replay_only"]),
            toy("toy-x", "integration", &["part"]),
            toy("toy-ax", "integration", &approx_only),
            bench(&["c=counter", "box=toy-x", "ax=toy-ax"], "poke", "{}"),
        ],
    )
}

/// The approximate plan: per part in world order, with an estimate for the replay only.
fn assert_approximate_plan(v: &Value) {
    let parts = v["parts"].as_array().unwrap();
    let hows: Vec<(&str, &str)> = parts
        .iter()
        .map(|p| (p["part"].as_str().unwrap(), p["how"].as_str().unwrap()))
        .collect();
    assert_eq!(
        hows,
        [("c", "replay"), ("box", "save"), ("ax", "approximate")]
    );
    let estimate = parts[0]["estimate_seconds"].as_f64();
    assert!(estimate.is_some_and(|s| s >= 0.0), "{v}");
    assert!(parts[1..]
        .iter()
        .all(|p| p.get("estimate_seconds").is_none()));
}

/// An unknown branch is a 404; an unknown step or restore a 400.
async fn assert_plan_bad_requests(fx: &Fx, branch: &str, at: i64) {
    let plan = |b: &str, q: &str| format!("/api/branches/{b}/restore-plan?{q}");
    let cases = [
        (plan("nope", &format!("at_seq={at}")), 404),
        (plan(branch, "at_seq=999"), 400),
        (plan(branch, &format!("at_seq={at}&restore=maybe")), 400),
    ];
    for (path, want) in cases {
        assert_eq!(fx.get(&path).await.0, want, "{path}");
    }
}

#[test]
fn restore_plan_endpoint() {
    run(async {
        let poke = |p: &str| call(&format!("{p}poke"), json!({}));
        let script = vec![add(5), poke(""), poke("ax_"), add(6), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("plan", plan_world(), &up).await;
        let (branch, evs) = fx.run_to_end().await;
        let at = model_call(&evs, 3);
        let path = |q: &str| format!("/api/branches/{branch}/restore-plan?at_seq={at}{q}");
        for restore in ["", "&restore=exact"] {
            let (st, v) = fx.get(&path(restore)).await;
            assert_eq!(st, 409, "{v}");
            let msg = v["error"].as_str().unwrap();
            assert!(msg.contains("ax") && msg.contains("approximate"), "{msg}");
        }
        let (st, v) = fx.get(&path("&restore=approximate")).await;
        assert_eq!(st, 200, "{v}");
        assert_approximate_plan(&v);
        assert_plan_bad_requests(&fx, &branch, at).await;
    })
}
