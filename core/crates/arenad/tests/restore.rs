//! Rerunning worlds whose parts restore in different ways: exact and approximate saves, replay,
//! fingerprints, the arena clock and the restore plan. The worlds are made of the kit's `counter`
//! example (in its restore flavours) and toy parts. Each task's setup (the world's own calls,
//! made before any agent) puts the world in the state a fork must restore, and the agents are
//! real models (see `support/live.rs`) that mostly only read, so what is restored doesn't depend
//! on what they chose to do. Every test here is `live_`.
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
    time::Duration,
};

use arenad::{app::AppState, model::*, runner, store::Store};
use live::Live;
use packages::{counter, package_root, toy, Package, TestHub, ToyRoot};
use parts::Hub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

/// One call of a task's setup: `tool` of part `part` with `args`.
fn call(part: &str, tool: &str, args: Value) -> String {
    json!({"part": part, "tool": tool, "args": args}).to_string()
}

fn add(n: i64) -> String {
    call("c", "add", json!({"n": n}))
}

/// The toy benchmark over `parts` (`name=integration`), whose gold is `gold` with `gold_args`,
/// and whose task `t1` sets up with `setup`.
fn bench(parts: &[&str], gold: &str, gold_args: &str, setup: &[String]) -> Package {
    bench_with(parts, gold, gold_args, setup, &[])
}

/// `bench` with `extra` benchmark args (e.g. `--config c={..}`).
fn bench_with(parts: &[&str], gold: &str, gold_args: &str, setup: &[String], extra: &[&str]) -> Package {
    let mut args = vec!["bench", "--gold", gold, "--gold-args", gold_args];
    for p in parts {
        args.extend(["--part", p]);
    }
    for s in setup {
        args.extend(["--setup", s.as_str()]);
    }
    args.extend_from_slice(extra);
    toy("toybench", "benchmark", &args)
}

/// A world of one part `c`: the counter in `flavour` (plus `extra` args), set up with `setup`.
fn counter_world(tag: &str, flavour: &str, extra: &[&str], setup: &[String]) -> ToyRoot {
    let mut args = vec!["--flavour", flavour];
    args.extend_from_slice(extra);
    package_root(
        tag,
        &[
            counter("counter", &args),
            bench(&["c=counter"], "add", r#"{"n": 1}"#, setup),
        ],
    )
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    hub: TestHub,
    live: Live,
    _root: ToyRoot,
    _clean: Cleanup,
}

/// An app served on a free port whose benchmark is `toybench` on `hub`, with the real drivers.
async fn serve_app(store: Arc<Store>, hub: Arc<Hub>) -> (Arc<AppState>, String) {
    app_support::app_for(store, hub, Some("toybench"), |_| {}).await
}

async fn setup(tag: &str, root: ToyRoot, live: &Live) -> Fx {
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("restore", tag).await;
    let (state, base) = serve_app(store, hub.hub.clone()).await;
    Fx {
        state,
        base,
        hub,
        live: live.clone(),
        _root: root,
        _clean: clean,
    }
}

/// `n` agents, one after another, that only read: each calls `tool` once.
fn readers(live: &Live, n: usize, tool: &str) -> Team {
    let prompt = format!("Call the {tool} tool once, then stop.");
    let ids = ["first", "second", "third"];
    live.team(ids[..n].iter().map(|id| live.reader(id, &prompt)).collect())
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(reqwest::Client::new().get(format!("{}{path}", self.base))).await
    }

    async fn fork(&self, branch: &str, at_seq: i64, restore: Option<&str>) -> (u16, Value) {
        fork_on(&self.base, branch, at_seq, restore).await
    }

    /// Runs `team` on task `t1` to the end; the branch and its events.
    async fn run_to_end(&self, team: Team) -> (String, Vec<Event>) {
        let (_run, branch) = runner::start_run(&self.state, "t1", team).await.unwrap();
        let (b, evs) = live::ended_ok(&self.state, &branch).await;
        live::assert_recorded(&b, &evs, &self.live);
        (branch, evs)
    }

    /// Forks `parent` at `at` with `restore` (200) and waits for the child (not
    /// `infra_error`): its id and events.
    async fn fork_to_end(&self, parent: &str, at: i64, restore: Option<&str>) -> (String, Vec<Event>) {
        let (st, v) = self.fork(parent, at, restore).await;
        assert_eq!(st, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        let (_b, evs) = live::ended_ok(&self.state, &child).await;
        (child, evs)
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

/// The seq of agent `id`'s start.
fn start_of(evs: &[Event], id: &str) -> i64 {
    live::first_of(evs, EventKind::AgentStart, id).unwrap()
}

fn tool_calls(evs: &[Event]) -> Vec<&Event> {
    evs.iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .collect()
}

/// The `value` results of the branch's `get` calls at or after `from`.
fn gets(evs: &[Event], from: i64) -> Vec<i64> {
    tool_calls(evs)
        .into_iter()
        .filter(|e| e.seq >= from && e.payload["name"] == "get")
        .filter_map(|e| e.payload["result"]["value"].as_i64())
        .collect()
}

/// What the child's agent read went on from the `from` the setup left (the clock adds one per
/// second on top, so more is fine; a fresh counter would be near 0).
fn assert_continued(cevs: &[Event], at: i64, from: i64) {
    for v in gets(cevs, at) {
        assert!((from..from + 3600).contains(&v), "{v} after {from}");
    }
}

/// A part with only approximate saves can't be restored exactly: the refusal names it, and
/// nothing is made.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_exact_refused_names_part() {
    let Some(live) = Live::any("live_exact_refused_names_part") else {
        return;
    };
    run(async {
        let part = ["part", "--exact", "none", "--approximate"];
        let root = package_root(
            "exactno",
            &[
                toy("toy-ax", "integration", &part),
                bench(&["box=toy-ax"], "poke", "{}", &[call("box", "poke", json!({}))]),
            ],
        );
        let fx = setup("exactno", root, &live).await;
        let (branch, evs) = fx.run_to_end(readers(&live, 1, "peek")).await;
        let at = start_of(&evs, "first");
        for restore in [None, Some("exact")] {
            let (st, v) = fx.fork(&branch, at, restore).await;
            assert_eq!(st, 409, "{v}");
            let msg = v["error"].as_str().unwrap();
            assert!(msg.contains("box") && msg.contains("approximate"), "{msg}");
        }
        let runs = fx.state.store.runs().await.unwrap();
        assert_eq!(runs[0].branches.len(), 1);
    })
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_approximate_marks_branch() {
    let Some(live) = Live::any("live_approximate_marks_branch") else {
        return;
    };
    run(async {
        let root = counter_world("approx", "approx_and_replay", &[], &[add(1000), add(7)]);
        let fx = setup("approx", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let at = start_of(&evs, "first");
        let (child, cevs) = fx.fork_to_end(&parent, at, Some("approximate")).await;
        assert_continued(&cevs, at, 1007);
        assert_restored(&fx, &child, "approximate", "approximate").await;
        assert_listed(&fx, &parent, &child).await;
    })
}

/// A world of replay-only parts can't be checked: the branch fails saying so, and its part is
/// listed as unchecked (and as not compared).
async fn assert_nothing_checked(fx: &Fx, parent: &str, child: &str) {
    let b = fx.state.store.branch(child).await.unwrap();
    assert_eq!(b.status, BranchStatus::Failed);
    assert_eq!(b.error.as_deref(), Some(arenad::runner::NOTHING_CHECKED));
    let evs = fx.state.store.events(child).await.unwrap();
    let check = live::check_of(&evs).unwrap();
    assert_eq!(check.unchecked, ["c"]);
    assert!(!check.passed);
    let (st, v) = fx.get(&format!("/api/compare?a={parent}&b={child}")).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(
        (&v["not_compared"], &v["differing_rows"]),
        (&json!(["c"]), &json!([]))
    );
}

#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_replay_only_restores_exactly() {
    let Some(live) = Live::any("live_replay_only_restores_exactly") else {
        return;
    };
    run(async {
        let root = counter_world("replay", "replay_only", &[], &[add(1000), add(7)]);
        let fx = setup("replay", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        // A part with only replay is never saved.
        assert!(tool_calls(&evs).iter().all(|e| e.payload["saves"].is_null()));
        let at = start_of(&evs, "first");
        let (child, cevs) = fx.fork_to_end(&parent, at, None).await;
        assert_continued(&cevs, at, 1007);
        assert_restored(&fx, &child, "exact", "replay").await;
        assert_nothing_checked(&fx, &parent, &child).await;
    })
}

/// A part with replay and approximate saves starts fresh with the run's seed in the run's first
/// branch too (the counter starts at `seed % 1000`), so an exact replay starts from the same
/// state and doesn't diverge.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_exact_replay_starts_like_the_original() {
    let Some(live) = Live::any("live_exact_replay_starts_like_the_original") else {
        return;
    };
    run(async {
        let root = counter_world("seeded", "approx_and_replay", &["--seeded"], &[add(1000)]);
        let fx = setup("seeded", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let fresh = (evs[0].payload["seed"].as_u64().unwrap() % 1000) as i64;
        let first = tool_calls(&evs)[0].payload["result"]["value"].as_i64().unwrap();
        assert!((fresh + 1000..fresh + 1003).contains(&first), "{fresh} {first}");
        let at = start_of(&evs, "first");
        let (child, cevs) = fx.fork_to_end(&parent, at, None).await;
        assert_continued(&cevs, at, first);
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_replay_divergence_fails() {
    let Some(live) = Live::any("live_replay_divergence_fails") else {
        return;
    };
    run(async {
        let root = counter_world("noisy", "nondeterministic", &[], &[add(1), add(2), add(3)]);
        let fx = setup("noisy", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let first_add = tool_calls(&evs)[0].seq;
        let (st, v) = fx.fork(&parent, start_of(&evs, "first"), None).await;
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_fingerprint_mismatch_refused() {
    let Some(live) = Live::any("live_fingerprint_mismatch_refused") else {
        return;
    };
    run(async {
        let setup_calls = [add(1), add(2)];
        let root = counter_world("fpa", "replay_only", &["--fingerprint", "a"], &setup_calls);
        let other = counter_world("fpb", "replay_only", &["--fingerprint", "b"], &setup_calls);
        let fx = setup("fp", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        // The same store and saves, with the counter now on another machine.
        let hub = fx.hub.sibling(std::slice::from_ref(&other.0));
        let (_state, base) = serve_app(fx.state.store.clone(), hub).await;
        let (st, v) = fork_on(&base, &parent, start_of(&evs, "first"), None).await;
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
            bench(&["box=toy-clock"], "poke", "{}", &[]),
        ],
    )
}

/// Each call saved the clock part (its advance reported a change) with the clock paused, so the
/// clock moved less than real time between them.
fn assert_paused_saves(first: &Event, second: &Event) {
    for c in [first, second] {
        assert!(c.payload["saves"]["box"].is_string(), "{}", c.payload);
        let paused = c.payload["paused_ms"].as_i64().unwrap();
        assert!(paused >= 350, "{}", c.payload);
    }
    let clock = |e: &Event| e.payload["clock_ms"].as_i64().unwrap();
    let real = (second.at - first.at).num_milliseconds();
    let moved = clock(second) - clock(first);
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_clock_advances_and_pauses_during_save() {
    let Some(live) = Live::any("live_clock_advances_and_pauses_during_save") else {
        return;
    };
    run(async {
        let logs = ToyRoot(packages::toy_dir("clocklog"));
        std::fs::create_dir_all(&logs.0).unwrap();
        let log = logs.0.join("calls.log");
        let fx = setup("clock", clock_world(&log), &live).await;
        let team = live.team(vec![live.agent("solo", "Call the poke tool once, then the peek tool once, then stop.")]);
        let (branch, evs) = fx.run_to_end(team).await;
        assert_world_event(&evs);
        assert_ticked_then_stopped(&log, &branch).await;
        let calls: Vec<&Event> = tool_calls(&evs).into_iter().filter(|e| e.payload["error"].is_null()).collect();
        match calls.as_slice() {
            [first, second, ..] => assert_paused_saves(first, second),
            _ => eprintln!("note: the model made under two calls; the paused saves were not exercised"),
        }
    })
}

/// A world of three parts: `c` replay only, `box` exact saves, `ax` approximate saves only, each
/// written by the setup.
fn plan_world() -> ToyRoot {
    let approx_only = [
        "part",
        "--exact",
        "none",
        "--approximate",
        "--prefix",
        "ax_",
    ];
    let setup = [add(5), call("box", "poke", json!({})), call("ax", "ax_poke", json!({})), add(6)];
    package_root(
        "plan",
        &[
            counter("counter", &["--flavour", "replay_only"]),
            toy("toy-x", "integration", &["part"]),
            toy("toy-ax", "integration", &approx_only),
            bench(&["c=counter", "box=toy-x", "ax=toy-ax"], "poke", "{}", &setup),
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_restore_plan_endpoint() {
    let Some(live) = Live::any("live_restore_plan_endpoint") else {
        return;
    };
    run(async {
        let fx = setup("plan", plan_world(), &live).await;
        let (branch, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let at = start_of(&evs, "first");
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_start_fingerprint_refusal_is_409() {
    let Some(live) = Live::any("live_start_fingerprint_refusal_is_409") else {
        return;
    };
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
                bench(&["box=toy-mv"], "poke", "{}", &[call("box", "poke", json!({}))]),
            ],
        );
        let fx = setup("moved", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "peek")).await;
        std::fs::write(&moved, "").unwrap();
        let (st, v) = fx.fork(&parent, start_of(&evs, "first"), None).await;
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_fork_clock_starts_at_fork_point() {
    let Some(live) = Live::any("live_fork_clock_starts_at_fork_point") else {
        return;
    };
    run(async {
        let root = counter_world("forkclock", "exact", &[], &[add(1)]);
        let fx = setup("forkclock", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 2, "get")).await;
        let clock = |e: &Event| e.payload["clock_ms"].as_i64();
        let at = start_of(&evs, "second");
        let fork_clock = evs.iter().filter(|e| e.seq < at).filter_map(clock).last().unwrap();
        let (_child, cevs) = fx.fork_to_end(&parent, at, None).await;
        match tool_calls(&cevs).into_iter().find(|e| e.seq >= at) {
            Some(first) => assert!(clock(first).unwrap() >= fork_clock, "{:?} < {fork_clock}", clock(first)),
            None => eprintln!("note: the child's agent made no call; its clock was not seen"),
        }
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
            bench(&["c=counter", "box=toy-slow"], "add", r#"{"n": 1}"#, &[add(1)]),
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_scoring_freezes_the_world() {
    let Some(live) = Live::any("live_scoring_freezes_the_world") else {
        return;
    };
    run(async {
        let fx = setup("freeze", freeze_world(), &live).await;
        let (_branch, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let late = late_world_events(&evs);
        assert!(late.is_empty(), "world events after the last agent: {late:?}");
        let check = live::check_of(&evs).unwrap();
        let final_c = check.final_saves.unwrap()["c"].clone();
        let start = start_of(&evs, "first");
        match gets(&evs, start).last() {
            Some(read) => assert_eq!(counter_in(&fx, &final_c).await, *read),
            None => eprintln!("note: the agent did not read; the frozen value was not compared"),
        }
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
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_approximate_before_the_first_save_replays() {
    let Some(live) = Live::any("live_approximate_before_the_first_save_replays") else {
        return;
    };
    run(async {
        let root = counter_world("firstsave", "approx_and_replay", &[], &[call("c", "get", json!({}))]);
        let fx = setup("firstsave", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let first = tool_calls(&evs)[0];
        assert!(first.payload["saves"]["c"].is_null(), "{}", first.payload);
        let at = start_of(&evs, "first");
        let (st, v) = restore_plan(&fx, &parent, at, "approximate").await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["parts"][0]["how"], "replay", "{v}");
        let (child, _) = fx.fork_to_end(&parent, at, Some("approximate")).await;
        assert_restored(&fx, &child, "approximate", "replay").await;
    })
}

/// A branch B restored its part approximately: an exact rerun of B past B's start would replay
/// the part through that approximate state, so it is refused (and the restore plan says so); an
/// approximate one works.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_exact_refused_after_an_approximate_restore() {
    let Some(live) = Live::any("live_exact_refused_after_an_approximate_restore") else {
        return;
    };
    run(async {
        let root = counter_world("approxchain", "approx_and_replay", &[], &[add(1000), add(7)]);
        let fx = setup("approxchain", root, &live).await;
        let (a, evs) = fx.run_to_end(readers(&live, 2, "get")).await;
        let (b, bevs) = fx.fork_to_end(&a, start_of(&evs, "first"), Some("approximate")).await;
        let b_start = fx.state.store.branch(&b).await.unwrap().start_seq.unwrap();
        let at = bevs
            .iter()
            .filter(|e| e.kind == EventKind::AgentStart && e.agent.as_deref() == Some("second"))
            .map(|e| e.seq)
            .find(|s| *s > b_start)
            .expect("B ran its second agent");
        let want = format!("c was restored approximately at step {b_start} in an earlier rerun, so it can't be restored exactly; choose Approximate");
        let (st, v) = fx.fork(&b, at, Some("exact")).await;
        assert_eq!((st, v["error"].as_str()), (409, Some(want.as_str())), "{v}");
        let (st, v) = restore_plan(&fx, &b, at, "exact").await;
        assert_eq!((st, v["error"].as_str()), (409, Some(want.as_str())), "{v}");
        fx.fork_to_end(&b, at, Some("approximate")).await;
    })
}

/// A replayed part whose config changed since the run can't be replayed exactly.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_exact_refused_when_the_config_changed() {
    let Some(live) = Live::any("live_exact_refused_when_the_config_changed") else {
        return;
    };
    run(async {
        let setup_calls = [add(1), add(2)];
        let root = counter_world("cfga", "replay_only", &[], &setup_calls);
        let config = ["--config", r#"c={"start": 5}"#];
        let changed = package_root(
            "cfgb",
            &[
                counter("counter", &["--flavour", "replay_only"]),
                bench_with(&["c=counter"], "add", r#"{"n": 1}"#, &setup_calls, &config),
            ],
        );
        let fx = setup("cfg", root, &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 1, "get")).await;
        let hub = fx.hub.sibling(std::slice::from_ref(&changed.0));
        let (_state, base) = serve_app(fx.state.store.clone(), hub).await;
        let (st, v) = fork_on(&base, &parent, start_of(&evs, "first"), None).await;
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
            bench(&["box=toy-alarm"], "poke", "{}", &[call("box", "poke", json!({}))]),
        ],
    )
}

/// A fork's clock parts restored from a save are moved to the fork point's clock before it
/// runs, and what they report on the way was recorded already: no world event twice.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_fork_reports_no_world_event_twice() {
    let Some(live) = Live::any("live_fork_reports_no_world_event_twice") else {
        return;
    };
    run(async {
        let fx = setup("alarm", alarm_world(), &live).await;
        let (parent, evs) = fx.run_to_end(readers(&live, 2, "peek")).await;
        let at = start_of(&evs, "second");
        let alarm = evs.iter().find(|e| e.kind == EventKind::WorldEvent).expect("an alarm");
        if alarm.seq > at {
            eprintln!("note: the alarm came after the first agent; nothing to report twice");
            return;
        }
        let (child, cevs) = fx.fork_to_end(&parent, at, None).await;
        let start = fx.state.store.branch(&child).await.unwrap().start_seq.unwrap();
        let again: Vec<&Event> = cevs
            .iter()
            .filter(|e| e.seq >= start && e.kind == EventKind::WorldEvent)
            .collect();
        assert!(again.is_empty(), "{again:?}");
    })
}
