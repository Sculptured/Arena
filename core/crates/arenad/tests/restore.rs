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
    providers::ProviderConfig,
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
            provider: None,
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
    bench_with(parts, gold, gold_args, &[])
}

/// `bench` with `extra` benchmark args (e.g. `--config c={..}`).
fn bench_with(parts: &[&str], gold: &str, gold_args: &str, extra: &[&str]) -> Package {
    let mut args = vec!["bench", "--gold", gold, "--gold-args", gold_args];
    for p in parts {
        args.extend(["--part", p]);
    }
    args.extend_from_slice(extra);
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
    config.providers = ProviderConfig::with_upstream(&up.base_url);
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

/// The branch's `Check` payload.
fn check_of(evs: &[Event]) -> &Value {
    let check = evs.iter().rev().find(|e| e.kind == EventKind::Check);
    &check.expect("a check").payload
}

/// A world of replay-only parts can't be checked: the branch fails saying so, and its part is
/// listed as unchecked (and as not compared).
async fn assert_nothing_checked(fx: &Fx, parent: &str, child: &str) {
    let b = fx.state.store.branch(child).await.unwrap();
    assert_eq!(b.status, BranchStatus::Failed);
    assert_eq!(b.error.as_deref(), Some(arenad::runner::NOTHING_CHECKED));
    let evs = fx.state.store.events(child).await.unwrap();
    assert_eq!(check_of(&evs)["unchecked"], json!(["c"]));
    assert_eq!(check_of(&evs)["passed"], false);
    let (st, v) = fx.get(&format!("/api/compare?a={parent}&b={child}")).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        (&v["not_compared"], &v["differing_rows"]),
        (&json!(["c"]), &json!([]))
    );
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
        assert_nothing_checked(&fx, &parent, &child).await;
    })
}

/// The `value` results of the branch's `get` calls at or after `from`.
fn gets(evs: &[Event], from: i64) -> Vec<i64> {
    tool_calls(evs)
        .into_iter()
        .filter(|e| e.seq >= from && e.payload["name"] == "get")
        .filter_map(|e| e.payload["result"]["value"].as_i64())
        .collect()
}

/// A part with replay and approximate saves starts fresh with the run's seed in the run's first
/// branch too, so an exact replay starts from the same state (the counter starts at
/// `seed % 1000`) and doesn't diverge.
#[test]
fn exact_replay_starts_like_the_original() {
    run(async {
        let root = counter_world("seeded", "approx_and_replay", &["--seeded"]);
        let script = vec![add(1000), get(), add(7), done(), get(), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("seeded", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let fresh = (evs[0].payload["seed"].as_u64().unwrap() % 1000) as i64;
        let first = tool_calls(&evs)[0].payload["result"]["value"]
            .as_i64()
            .unwrap();
        assert!(
            (fresh + 1000..fresh + 1003).contains(&first),
            "{fresh} {first}"
        );
        let at = model_call(&evs, 2);
        let (st, v) = fx.fork(&parent, at, None).await;
        assert_eq!(st, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        fx.finished(&child).await;
        let before = gets(&evs, 0)[0];
        let after = gets(&fx.state.store.events(&child).await.unwrap(), at)[0];
        assert!((before..before + 3).contains(&after), "{before} -> {after}");
        assert_restored(&fx, &child, "exact", "replay").await;
    })
}

/// The fork of `parent`: its only child.
async fn only_child(fx: &Fx, parent: &str) -> Branch {
    let runs = fx.state.store.runs().await.unwrap();
    let children: Vec<Branch> = runs[0]
        .branches
        .iter()
        .filter(|b| b.parent_id.as_deref() == Some(parent))
        .cloned()
        .collect();
    assert_eq!(children.len(), 1);
    children[0].clone()
}

/// The counter program no longer has the instance of part `c` of `branch`.
async fn assert_stopped(fx: &Fx, branch: &str) {
    let instance = arenad::world::instance_id(branch, "c");
    let c = fx.state.hub.client("counter").await.unwrap();
    let params = json!({"target": {"instance": instance}, "keys": []});
    match c.call::<Value>("inspect", params).await {
        Err(parts::CallError::Part { code: -32002, .. }) => {}
        other => panic!("the replay instance still runs: {other:?}"),
    }
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
        // The new branch ended infra_error with that message, its instance stopped.
        let child = only_child(&fx, &parent).await;
        assert_eq!(child.status, BranchStatus::InfraError);
        assert_eq!(child.error.as_deref(), Some(want.as_str()));
        assert_stopped(&fx, &child.id).await;
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

/// A part refusing to start with a fingerprint mismatch (-32003) is the same 409, and the new
/// branch ends infra_error.
#[test]
fn start_fingerprint_refusal_is_409() {
    run(async {
        let marks = ToyRoot(packages::toy_dir("movedmark"));
        std::fs::create_dir_all(&marks.0).unwrap();
        let moved = marks.0.join("moved");
        let moved_s = moved.to_string_lossy().into_owned();
        let part = ["part", "--refuse-start-if", &moved_s];
        let root = package_root(
            "moved",
            &[
                toy("toy-mv", "integration", &part),
                bench(&["box=toy-mv"], "poke", "{}"),
            ],
        );
        let poke = call("poke", json!({}));
        let up = FakeUpstream::start(vec![poke.clone(), poke, done()]).await;
        let fx = setup("moved", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        std::fs::write(&moved, "").unwrap();
        let (st, v) = fx.fork(&parent, model_call(&evs, 1), None).await;
        assert_eq!(st, 409, "{v}");
        let want =
            "the box integration is running on a different machine or version than the original";
        assert_eq!(v["error"], want);
        assert_eq!(
            only_child(&fx, &parent).await.status,
            BranchStatus::InfraError
        );
    })
}

/// A fork's clock starts at the fork point's clock (the last one recorded before it), not 0.
#[test]
fn fork_clock_starts_at_fork_point() {
    run(async {
        let root = counter_world("forkclock", "exact", &[]);
        let mut slow = add(2);
        slow["_delay_ms"] = json!(1500);
        let script = vec![add(1), slow, add(3), done(), get(), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("forkclock", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let clock = |e: &Event| e.payload["clock_ms"].as_i64().unwrap();
        let fork_clock = clock(tool_calls(&evs)[1]);
        assert!(fork_clock >= 1400, "{fork_clock}");
        let at = model_call(&evs, 2);
        let (st, v) = fx.fork(&parent, at, None).await;
        assert_eq!(st, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        fx.finished(&child).await;
        let cevs = fx.state.store.events(&child).await.unwrap();
        let first = tool_calls(&cevs).into_iter().find(|e| e.seq >= at).unwrap();
        assert!(
            clock(first) >= fork_clock,
            "{} < {fork_clock}",
            clock(first)
        );
    })
}

/// The value the counter keeps in `save`.
async fn counter_in(fx: &Fx, save: &str) -> i64 {
    let c = fx.state.hub.client("counter").await.unwrap();
    let params = json!({"target": {"save": save}, "keys": ["counter/value"]});
    let r: Value = c.call("inspect", params).await.unwrap();
    r["docs"]["counter/value"].as_i64().unwrap()
}

/// A world of a counter `c` (exact saves) and a toy clock part whose saves take 1.5 s and that
/// reports an event on every advance.
fn freeze_world() -> ToyRoot {
    let slow = [
        "part",
        "--uses-clock",
        "--event-every",
        "--save-sleep",
        "1500",
    ];
    package_root(
        "freeze",
        &[
            counter("counter", &["--flavour", "exact"]),
            toy("toy-slow", "integration", &slow),
            bench(&["c=counter", "box=toy-slow"], "add", r#"{"n": 1}"#),
        ],
    )
}

/// The world events recorded after the last agent's end.
fn late_world_events(evs: &[Event]) -> Vec<i64> {
    let end = evs
        .iter()
        .rev()
        .find(|e| e.kind == EventKind::AgentEnd)
        .unwrap()
        .seq;
    evs.iter()
        .filter(|e| e.kind == EventKind::WorldEvent && e.seq > end)
        .map(|e| e.seq)
        .collect()
}

/// Once the last agent ends the world stands still: no world event after it, and the counter's
/// final save is what the agent last read, however long the gold save takes to build.
#[test]
fn scoring_freezes_the_world() {
    run(async {
        let up = FakeUpstream::start(vec![add(1), get(), done()]).await;
        let fx = setup("freeze", freeze_world(), &up).await;
        let (_branch, evs) = fx.run_to_end().await;
        let late = late_world_events(&evs);
        assert!(
            late.is_empty(),
            "world events after the last agent: {late:?}"
        );
        let final_c = check_of(&evs)["final_saves"]["c"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(counter_in(&fx, &final_c).await, gets(&evs, 0)[0]);
    })
}

/// The restore plan of a fork of `branch` at `at` with `restore`.
async fn restore_plan(fx: &Fx, branch: &str, at: i64, restore: &str) -> (u16, Value) {
    let path = format!("/api/branches/{branch}/restore-plan?at_seq={at}&restore={restore}");
    fx.get(&path).await
}

/// A part with replay and approximate saves starts fresh in the run's first branch, so it has no
/// save before its first one: an approximate fork before it replays the part (exactly), and the
/// restore plan says so.
#[test]
fn approximate_before_the_first_save_replays() {
    run(async {
        let root = counter_world("firstsave", "approx_and_replay", &[]);
        let script = vec![get(), add(1000), done(), get(), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("firstsave", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let first = tool_calls(&evs)[0];
        assert!(first.payload["saves"]["c"].is_null(), "{}", first.payload);
        let at = model_call(&evs, 1);
        let (st, v) = restore_plan(&fx, &parent, at, "approximate").await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["parts"][0]["how"], "replay", "{v}");
        let (st, v) = fx.fork(&parent, at, Some("approximate")).await;
        assert_eq!(st, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        let b = fx.finished(&child).await;
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        assert_restored(&fx, &child, "approximate", "replay").await;
    })
}

/// A branch B restored its part approximately: an exact rerun of B past B's start would replay
/// the part through that approximate state, so it is refused (and the restore plan says so); an
/// approximate one works.
#[test]
fn exact_refused_after_an_approximate_restore() {
    run(async {
        let root = counter_world("approxchain", "approx_and_replay", &[]);
        let script = vec![add(1000), add(7), done(), get(), done(), done()];
        let up = FakeUpstream::start(script).await;
        let fx = setup("approxchain", root, &up).await;
        let (a, evs) = fx.run_to_end().await;
        let b = fork_and_finish(&fx, &a, model_call(&evs, 1), Some("approximate")).await;
        let b_start = fx.state.store.branch(&b).await.unwrap().start_seq.unwrap();
        let bevs = fx.state.store.events(&b).await.unwrap();
        let at = model_call(&bevs, 2);
        assert!(at > b_start, "{at} <= {b_start}");
        let want = format!("c was restored approximately at step {b_start} in an earlier rerun, so it can't be restored exactly; choose Approximate");
        let (st, v) = fx.fork(&b, at, Some("exact")).await;
        assert_eq!((st, v["error"].as_str()), (409, Some(want.as_str())), "{v}");
        let (st, v) = restore_plan(&fx, &b, at, "exact").await;
        assert_eq!((st, v["error"].as_str()), (409, Some(want.as_str())), "{v}");
        let (st, v) = fx.fork(&b, at, Some("approximate")).await;
        assert_eq!(st, 200, "{v}");
        let c = fx.finished(v["branch_id"].as_str().unwrap()).await;
        assert_ne!(c.status, BranchStatus::InfraError, "{:?}", c.error);
    })
}

/// A replayed part whose config changed since the run can't be replayed exactly.
#[test]
fn exact_refused_when_the_config_changed() {
    run(async {
        let root = counter_world("cfga", "replay_only", &[]);
        let config = ["--config", r#"c={"start": 5}"#];
        let changed = package_root(
            "cfgb",
            &[
                counter("counter", &["--flavour", "replay_only"]),
                bench_with(&["c=counter"], "add", r#"{"n": 1}"#, &config),
            ],
        );
        let up = FakeUpstream::start(vec![add(1), add(2), done()]).await;
        let fx = setup("cfg", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let hub = fx.hub.sibling(std::slice::from_ref(&changed.0));
        let (_state, base) = serve_app(fx.state.store.clone(), hub, &up).await;
        let (st, v) = fork_on(&base, &parent, model_call(&evs, 1), None).await;
        assert_eq!(st, 409, "{v}");
        let want = "the part c's config changed since the run, so it can't be restored exactly; choose Approximate";
        assert_eq!(v["error"], want);
    })
}

/// A world of one toy clock part `box` (exact saves) that reports an alarm on an instance's
/// first advance.
fn alarm_world() -> ToyRoot {
    package_root(
        "alarm",
        &[
            toy("toy-alarm", "integration", &["part", "--uses-clock"]),
            bench(&["box=toy-alarm"], "poke", "{}"),
        ],
    )
}

/// A fork's clock parts restored from a save are moved to the fork point's clock before it
/// runs, and what they report on the way was recorded already: no world event twice.
#[test]
fn fork_reports_no_world_event_twice() {
    run(async {
        let (poke, peek) = (call("poke", json!({})), call("peek", json!({})));
        let up = FakeUpstream::start(vec![poke, peek.clone(), done(), peek, done()]).await;
        let fx = setup("alarm", alarm_world(), &up).await;
        let (parent, evs) = fx.run_to_end().await;
        assert!(evs.iter().any(|e| e.kind == EventKind::WorldEvent));
        let (st, v) = fx.fork(&parent, model_call(&evs, 1), None).await;
        assert_eq!(st, 200, "{v}");
        let child = fx
            .state
            .store
            .branch(v["branch_id"].as_str().unwrap())
            .await
            .unwrap();
        let b = fx.finished(&child.id).await;
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        let start = child.start_seq.unwrap();
        let cevs = fx.state.store.events(&child.id).await.unwrap();
        let again: Vec<&Event> = cevs
            .iter()
            .filter(|e| e.seq >= start && e.kind == EventKind::WorldEvent)
            .collect();
        assert!(again.is_empty(), "{again:?}");
    })
}
