//! What a benchmark wires into its world. Links: an event of one part calls a tool of another,
//! recorded as a step of the actor `link` and repeated from the record by a replay. Gates: a tool
//! is held back unless a document of another part allows it. And named checks over the end state.
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
    world::plan,
};
use fake_upstream::FakeUpstream;
use packages::{counter, package_root, toy, Package, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn solo() -> Team {
    let agent = AgentSpec {
        id: "solo".into(),
        name: "solo".into(),
        kind: AgentKind::Open,
        model: Some("fake/model".into()),
        prompt: "You are solo.".into(),
        provider: None,
        read_only: false,
    };
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent],
    }
}

fn call(name: &str) -> Value {
    let tc = json!({"id": "c", "type": "function", "function": {"name": name, "arguments": "{}"}});
    json!({"role": "assistant", "content": null, "tool_calls": [tc]})
}

fn done() -> Value {
    json!({"role": "assistant", "content": "Done."})
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
    let (store, clean) = app_support::meta_store("links", tag).await;
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

    async fn run_to_end(&self) -> (String, Vec<Event>) {
        let (_run, branch) = runner::start_run(&self.state, "t1", solo()).await.unwrap();
        let b = self.finished(&branch).await;
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        let events = self.state.store.events(&branch).await.unwrap();
        (branch, events)
    }

    async fn fork(&self, branch: &str, at_seq: i64) -> (u16, Value) {
        let url = format!("{}/api/branches/{branch}/fork", self.base);
        let body = json!({"at_seq": at_seq, "changes": {}});
        app_support::send(reqwest::Client::new().post(url).json(&body)).await
    }
}

fn bench(parts: &[&str], gold: &str, links: &[Value]) -> Package {
    let links: Vec<String> = links.iter().map(Value::to_string).collect();
    let mut args = vec!["bench", "--gold", gold, "--gold-args", r#"{"n": 1}"#];
    for p in parts {
        args.extend(["--part", p]);
    }
    for l in &links {
        args.extend(["--link", l]);
    }
    toy("toybench", "benchmark", &args)
}

/// A toy clock part `src` whose first advance reports an `alarm`, linked to `add` on the counter
/// `c` in `flavour`.
fn alarm_world(tag: &str, flavour: &str, n: Value) -> ToyRoot {
    let link = json!({"from": "src", "event": "alarm", "to": "c", "tool": "add", "args": {"n": n}});
    package_root(
        tag,
        &[
            toy("toy-src", "integration", &["part", "--uses-clock"]),
            counter("counter", &["--flavour", flavour]),
            bench(&["src=toy-src", "c=counter"], "poke", &[link]),
        ],
    )
}

/// Two toy parts: `a`'s `a_poke` gives an event `rung`, and `b` has `b_poke`.
fn rung_world(tag: &str, links: &[Value]) -> ToyRoot {
    package_root(
        tag,
        &[
            toy(
                "toy-a",
                "integration",
                &["part", "--prefix", "a_", "--tool-event", "rung"],
            ),
            toy("toy-b", "integration", &["part", "--prefix", "b_"]),
            bench(&["a=toy-a", "b=toy-b"], "a_poke", links),
        ],
    )
}

fn link_steps(evs: &[Event]) -> Vec<&Event> {
    let is_link = |e: &&Event| e.kind == EventKind::ToolCall && e.agent.as_deref() == Some("link");
    evs.iter().filter(is_link).collect()
}

fn model_call(evs: &[Event], n: usize) -> i64 {
    evs.iter()
        .filter(|e| e.kind == EventKind::ModelCall)
        .nth(n)
        .unwrap()
        .seq
}

fn get_value(evs: &[Event], from: i64) -> i64 {
    evs.iter()
        .find(|e| e.seq >= from && e.kind == EventKind::ToolCall && e.payload["name"] == "get")
        .and_then(|e| e.payload["result"]["value"].as_i64())
        .unwrap_or_else(|| panic!("no get after {from}"))
}

fn assert_alarm_then_link(evs: &[Event]) {
    let steps = link_steps(evs);
    assert_eq!(steps.len(), 1, "{evs:?}");
    let step = &steps[0].payload;
    let alarm = evs
        .iter()
        .find(|e| e.kind == EventKind::WorldEvent)
        .unwrap();
    assert!(alarm.seq < steps[0].seq);
    assert_eq!(alarm.payload["kind"], "alarm");
    assert_eq!(step["clock_ms"], alarm.payload["clock_ms"]);
    assert_eq!(
        (&step["name"], &step["part"], &step["args"]),
        (&json!("add"), &json!("c"), &json!({"n": 1000}))
    );
    assert!(step["saves"]["src"].is_string(), "{step}");
}

/// An exact fork after the link's step gets its effect from the record and calls nothing again.
async fn assert_fork_replays_the_step(fx: &Fx, parent: &str, evs: &[Event]) {
    let (st, v) = fx.fork(parent, model_call(evs, 1)).await;
    assert_eq!(st, 200, "{v}");
    let child = v["branch_id"].as_str().unwrap();
    let b = fx.finished(child).await;
    assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
    let start = fx
        .state
        .store
        .branch(child)
        .await
        .unwrap()
        .start_seq
        .unwrap();
    let cevs = fx.state.store.events(child).await.unwrap();
    assert!(get_value(&cevs, start) >= 1000, "the replay added it again");
    let own: Vec<Event> = cevs.iter().filter(|e| e.seq >= start).cloned().collect();
    assert!(link_steps(&own).is_empty(), "{own:?}");
}

/// The alarm calls `add` as a `link` step at the alarm's time, agents don't get `add`, and an
/// exact fork replays the step from the record without calling it again.
#[test]
fn an_event_calls_the_linked_tool_and_a_replay_repeats_it() {
    run(async {
        let script = vec![call("get"), call("get"), done(), call("get"), done()];
        let up = FakeUpstream::start(script).await;
        let root = alarm_world("alarm", "replay_only", json!(1000));
        let fx = setup("alarm", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        assert_alarm_then_link(&evs);
        assert!(get_value(&evs, 0) >= 1000);
        let offered = up.requests()[0]["tools"].to_string();
        assert!(
            offered.contains("\"get\"") && !offered.contains("\"add\""),
            "{offered}"
        );

        assert_fork_replays_the_step(&fx, &parent, &evs).await;
    })
}

/// A link's call is an input of its part: what it gave must come out again on a replay.
#[test]
fn a_linked_call_that_replays_differently_diverges() {
    run(async {
        let up = FakeUpstream::start(vec![call("get"), call("get"), done()]).await;
        let root = alarm_world("noisy", "nondeterministic", json!(1));
        let fx = setup("noisy", root, &up).await;
        let (parent, evs) = fx.run_to_end().await;
        let seq = link_steps(&evs)[0].seq;
        let (st, v) = fx.fork(&parent, model_call(&evs, 1)).await;
        assert_eq!(st, 409, "{v}");
        let want = format!("replay of c diverged at step {seq}, so it can't be restored exactly");
        assert_eq!(v["error"], want.as_str());
    })
}

/// An argument the event lacks: the step says so and the part is not called.
#[test]
fn an_argument_the_event_lacks_is_a_step_error() {
    run(async {
        let up = FakeUpstream::start(vec![call("get"), done()]).await;
        let root = alarm_world("lacks", "replay_only", json!("/no/such"));
        let fx = setup("lacks", root, &up).await;
        let (_, evs) = fx.run_to_end().await;
        let steps = link_steps(&evs);
        let step = &steps[0].payload;
        assert!(step["part"].is_null(), "{step}");
        assert!(
            step["error"].as_str().unwrap().contains("no /no/such"),
            "{step}"
        );
    })
}

/// An event in a tool call's result sets off its link after the call's own step, and the linked
/// part is saved with the link's step.
#[test]
fn an_event_in_a_tool_result_calls_the_link() {
    run(async {
        let link = json!({"from": "a", "event": "rung", "to": "b", "tool": "b_poke"});
        let up = FakeUpstream::start(vec![call("a_poke"), done()]).await;
        let fx = setup("rung", rung_world("rung", &[link]), &up).await;
        let (_, evs) = fx.run_to_end().await;
        let poke = evs.iter().find(|e| e.payload["name"] == "a_poke").unwrap();
        let rung = evs
            .iter()
            .find(|e| e.kind == EventKind::WorldEvent)
            .unwrap();
        let steps = link_steps(&evs);
        assert!(poke.seq < rung.seq && rung.seq < steps[0].seq);
        assert_eq!(rung.payload["part"], "a");
        assert_eq!(steps[0].payload["clock_ms"], poke.payload["clock_ms"]);
        assert!(steps[0].payload["saves"]["b"].is_string(), "{evs:?}");
    })
}

/// Links that set each other off stop at the limit, with a step that says so.
#[test]
fn links_in_a_loop_stop() {
    run(async {
        let link = json!({"from": "a", "event": "alarm", "to": "a", "tool": "a_poke"});
        let part_args = [
            "part",
            "--prefix",
            "a_",
            "--uses-clock",
            "--tool-event",
            "alarm",
        ];
        let root = package_root(
            "loop",
            &[
                toy("toy-a", "integration", &part_args),
                toy("toy-b", "integration", &["part", "--prefix", "b_"]),
                bench(&["a=toy-a", "b=toy-b"], "b_poke", &[link]),
            ],
        );
        let up = FakeUpstream::start(vec![call("a_peek"), done()]).await;
        let fx = setup("loop", root, &up).await;
        let (_, evs) = fx.run_to_end().await;
        let steps = link_steps(&evs);
        assert_eq!(steps.len(), 65);
        let last = steps[64].payload["error"].as_str().unwrap();
        assert!(last.contains("64 calls in one step"), "{last}");
    })
}

/// A world whose link names something it doesn't have is refused.
#[test]
fn a_link_to_nothing_is_refused() {
    run(async {
        let links = [
            json!({"from": "nope", "event": "e", "to": "b", "tool": "b_poke"}),
            json!({"from": "a", "event": "e", "to": "nope", "tool": "b_poke"}),
            json!({"from": "a", "event": "e", "to": "b", "tool": "a_poke"}),
        ];
        for (i, link) in links.into_iter().enumerate() {
            let root = rung_world(&format!("bad{i}"), &[link]);
            let hub = TestHub::new(std::slice::from_ref(&root.0));
            let bench = hub.hub.bench_describe("toybench").await.unwrap();
            assert!(plan(&hub.hub, &bench).await.is_err());
        }
    })
}

fn gated_world(tag: &str) -> ToyRoot {
    let gate = json!({"tool": "a_poke", "part": "guard", "key": "box/n", "require": {"/n": 1}});
    let gate = gate.to_string();
    let args = [
        "bench",
        "--gold",
        "g_poke",
        "--part",
        "guard=toy-g",
        "--part",
        "act=toy-a",
        "--gate",
        &gate,
    ];
    package_root(
        tag,
        &[
            toy("toy-g", "integration", &["part", "--prefix", "g_"]),
            toy("toy-a", "integration", &["part", "--prefix", "a_"]),
            toy("toybench", "benchmark", &args),
        ],
    )
}

/// A gated call is refused until the other part's document allows it; a refused one is no input
/// of the part, so a replay would not repeat it.
#[test]
fn a_gate_holds_a_tool_back_until_the_document_allows_it() {
    run(async {
        let script = vec![
            call("a_poke"),
            call("g_poke"),
            call("a_poke"),
            call("a_peek"),
            done(),
        ];
        let up = FakeUpstream::start(script).await;
        let fx = setup("gated", gated_world("gated"), &up).await;
        let (_, evs) = fx.run_to_end().await;
        let pokes: Vec<&Event> = evs
            .iter()
            .filter(|e| e.payload["name"] == "a_poke")
            .collect();
        let refused = &pokes[0].payload;
        assert!(refused["part"].is_null(), "{refused}");
        assert!(
            refused["error"].as_str().unwrap().contains("/n"),
            "{refused}"
        );
        assert_eq!(pokes[1].payload["result"]["n"], 1);
        let peek = evs.iter().find(|e| e.payload["name"] == "a_peek").unwrap();
        assert_eq!(
            peek.payload["result"]["n"], 1,
            "the refused call changed nothing"
        );
    })
}

async fn assert_scored(tag: &str, script: Vec<Value>, status: BranchStatus, failed: Vec<String>) {
    let up = FakeUpstream::start(script).await;
    let args = [
        "bench",
        "--integration",
        "toy-box",
        "--gold",
        "nope",
        "--scored",
    ];
    let root = package_root(
        tag,
        &[
            toy("toy-box", "integration", &["part"]),
            toy("toybench", "benchmark", &args),
        ],
    );
    let fx = setup(tag, root, &up).await;
    let (branch, evs) = fx.run_to_end().await;
    assert_eq!(fx.finished(&branch).await.status, status);
    let check = evs.iter().find(|e| e.kind == EventKind::Check).unwrap();
    let c: CheckPayload = serde_json::from_value(check.payload.clone()).unwrap();
    assert_eq!(c.differing_rows, Some(failed));
    assert_eq!(c.checks.unwrap()[0].name, "poked");
    assert!(c.final_saves.unwrap().contains_key("box"));
}

/// A benchmark that scores itself gets the world's end state; a branch passes when every named
/// check does, and no gold world is made.
#[test]
fn a_scored_benchmark_checks_the_end_state() {
    run(async {
        let pass = vec![call("poke"), done()];
        assert_scored("scorepass", pass, BranchStatus::Passed, vec![]).await;
        let failed = vec!["poked".to_string()];
        assert_scored("scorefail", vec![done()], BranchStatus::Failed, failed).await;
    })
}
