//! What a benchmark wires into its world. Links: an event of one part calls a tool of another,
//! recorded as a step of the actor `link` and repeated from the record by a replay. Gates: a tool
//! is held back unless a document of another part allows it. A task's setup. And named checks
//! over the end state. Without a model: worlds that are refused, a setup that fails, and the
//! gate and a link set off by a call, checked by calling the world's tools through Arena's MCP
//! endpoint on a live branch. `live_`: runs on a real model (see `support/live.rs`), while the
//! clock, the setup and the links do what they do on their own.
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
};

use app_support::send;
use arenad::{app::AppState, exec::BranchCtx, model::*, runner, world::plan};
use live::Live;
use packages::{counter, package_root, toy, Package, TestHub, ToyRoot};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

/// Runs `body` on the live model, when there is one (else the test was skipped, said why).
fn run_live<F: Future<Output = ()>>(live: Option<Live>, body: impl FnOnce(Live) -> F) {
    if let Some(live) = live {
        run(body(live));
    }
}

/// A team for runs that never reach an agent, and for live branches driven through MCP.
fn idle_team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![AgentSpec {
            id: "solo".into(),
            name: "solo".into(),
            kind: AgentKind::ClaudeCode,
            model: Some("haiku".into()),
            prompt: "You are solo.".into(),
            provider: None,
            read_only: false,
        }],
    }
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

async fn setup(tag: &str, root: ToyRoot) -> Fx {
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("links", tag).await;
    let (state, base) =
        app_support::app_for(store, hub.hub.clone(), Some("toybench"), |_| {}).await;
    Fx {
        state,
        base,
        hub,
        _root: root,
        _clean: clean,
    }
}

impl Fx {
    /// Runs `team` on task `t1` to its end (not an infrastructure error): its branch and events.
    async fn run_to_end(&self, live: &Live, team: Team) -> (String, Vec<Event>) {
        let (_run, branch) = runner::start_run(&self.state, "t1", team).await.unwrap();
        let (b, evs) = live::ended_ok(&self.state, &branch).await;
        live::assert_recorded(&b, &evs, live);
        (branch, evs)
    }

    async fn fork(&self, branch: &str, at_seq: i64) -> (u16, Value) {
        let url = format!("{}/api/branches/{branch}/fork", self.base);
        let body = json!({"at_seq": at_seq, "changes": {}});
        send(reqwest::Client::new().post(url).json(&body)).await
    }

    /// A branch of `t1` whose world is started and live, as a run's is before its agents: its
    /// id and context. Nothing runs on it: the test calls its tools through Arena's MCP endpoint.
    async fn live_branch(&self) -> (String, Arc<BranchCtx>) {
        let team = idle_team();
        let run_id = self.state.store.create_run("t1", &team).await.unwrap();
        let branch = self
            .state
            .store
            .create_branch(&run_id, None, &team, &json!([]), None)
            .await
            .unwrap();
        let world = self.hub.start_world("toybench", &branch).await;
        let ctx = Arc::new(BranchCtx::new(branch.clone(), world, team, "t1".into()));
        self.state
            .live
            .write()
            .unwrap()
            .insert(branch.clone(), ctx.clone());
        (branch, ctx)
    }

    /// Calls `name` through Arena's MCP endpoint for agent `solo` on `branch`: the tool's result.
    async fn mcp(&self, branch: &str, name: &str) -> Value {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                          "params": {"name": name, "arguments": {}}});
        let url = format!("{}/mcp/{branch}/solo", self.base);
        let (status, v) = send(reqwest::Client::new().post(url).json(&body)).await;
        assert_eq!(status, 200, "{v}");
        v["result"].clone()
    }

    /// The tools agents get in this world.
    async fn agent_tools(&self) -> Vec<String> {
        let bench = self.hub.hub.bench_describe("toybench").await.unwrap();
        let planned = plan(&self.hub.hub, &bench).await.unwrap();
        planned.tools.into_iter().map(|(t, _)| t.name).collect()
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
    live::calls_by(evs, "link")
}

/// The `value` of each successful `get` from `from` on.
fn gets(evs: &[Event], from: i64) -> Vec<i64> {
    live::calls_named(evs, "get")
        .into_iter()
        .filter(|e| e.seq >= from)
        .filter_map(|e| e.payload["result"]["value"].as_i64())
        .collect()
}

/// Two reading agents, one after the other: the world's clock runs while the first works.
fn two_readers(live: &Live) -> Team {
    let read = "Call the get tool once, then stop.";
    live.team(vec![
        live.reader("first", read),
        live.reader("second", read),
    ])
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

/// The seq of the second agent's start, when the link's step came before it.
fn after_the_link(evs: &[Event]) -> Option<i64> {
    let start = live::first_of(evs, EventKind::AgentStart, "second")?;
    let link = link_steps(evs).first()?.seq;
    if link > start {
        eprintln!("note: the alarm came after the first agent; the fork part was not exercised");
        return None;
    }
    Some(start)
}

/// The alarm calls `add` as a `link` step at the alarm's time, agents don't get `add`, a read
/// after it sees what it added, and an exact fork after the link's step gets its effect from the
/// record and calls nothing again.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_an_event_calls_the_linked_tool_and_a_replay_repeats_it() {
    run_live(
        Live::any("live_an_event_calls_the_linked_tool_and_a_replay_repeats_it"),
        an_event_calls_the_linked_tool_and_a_replay_repeats_it,
    );
}

async fn an_event_calls_the_linked_tool_and_a_replay_repeats_it(live: Live) {
    let fx = setup("alarm", alarm_world("alarm", "replay_only", json!(1000))).await;
    let tools = fx.agent_tools().await;
    assert!(
        tools.iter().any(|t| t == "get") && !tools.iter().any(|t| t == "add"),
        "{tools:?}"
    );
    let (parent, evs) = fx.run_to_end(&live, two_readers(&live)).await;
    assert_alarm_then_link(&evs);
    let link = link_steps(&evs)[0].seq;
    assert!(
        gets(&evs, link).iter().all(|v| *v >= 1000),
        "{:?}",
        gets(&evs, link)
    );
    let Some(at) = after_the_link(&evs) else {
        return;
    };
    let (st, v) = fx.fork(&parent, at).await;
    assert_eq!(st, 200, "{v}");
    let child = v["branch_id"].as_str().unwrap();
    let (_b, cevs) = live::ended_ok(&fx.state, child).await;
    let own: Vec<Event> = cevs.iter().filter(|e| e.seq >= at).cloned().collect();
    assert!(link_steps(&own).is_empty(), "{own:?}");
    assert!(
        gets(&cevs, at).iter().all(|v| *v >= 1000),
        "the replay added it again"
    );
}

/// A link's call is an input of its part: what it gave must come out again on a replay, so a
/// part whose answers vary can't be restored past it.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_linked_call_that_replays_differently_diverges() {
    run_live(
        Live::any("live_a_linked_call_that_replays_differently_diverges"),
        a_linked_call_that_replays_differently_diverges,
    );
}

async fn a_linked_call_that_replays_differently_diverges(live: Live) {
    let fx = setup("noisy", alarm_world("noisy", "nondeterministic", json!(1))).await;
    let (parent, evs) = fx.run_to_end(&live, two_readers(&live)).await;
    let Some(at) = after_the_link(&evs) else {
        return;
    };
    let seq = link_steps(&evs)[0].seq;
    let (st, v) = fx.fork(&parent, at).await;
    assert_eq!(st, 409, "{v}");
    let want = format!("replay of c diverged at step {seq}, so it can't be restored exactly");
    assert_eq!(v["error"], want.as_str());
}

/// An argument the event lacks: the step says so and the part is not called.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_an_argument_the_event_lacks_is_a_step_error() {
    run_live(
        Live::any("live_an_argument_the_event_lacks_is_a_step_error"),
        an_argument_the_event_lacks_is_a_step_error,
    );
}

async fn an_argument_the_event_lacks_is_a_step_error(live: Live) {
    let fx = setup(
        "lacks",
        alarm_world("lacks", "replay_only", json!("/no/such")),
    )
    .await;
    let team = live.team(vec![
        live.reader("solo", "Call the get tool once, then stop.")
    ]);
    let (_, evs) = fx.run_to_end(&live, team).await;
    let steps = link_steps(&evs);
    let step = &steps[0].payload;
    assert!(step["part"].is_null(), "{step}");
    assert!(
        step["error"].as_str().unwrap().contains("no /no/such"),
        "{step}"
    );
}

/// An event in a tool call's result sets off its link after the call's own step, at the call's
/// time, and the linked part is saved with the link's step.
#[test]
fn an_event_in_a_tool_result_calls_the_link() {
    run(async {
        let link = json!({"from": "a", "event": "rung", "to": "b", "tool": "b_poke"});
        let fx = setup("rung", rung_world("rung", &[link])).await;
        let (branch, ctx) = fx.live_branch().await;
        assert_eq!(fx.mcp(&branch, "a_poke").await["isError"], false);
        let evs = fx.state.store.events(&branch).await.unwrap();
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
        ctx.world.stop().await;
    })
}

/// Links that set each other off stop at the limit, with a step that says so.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_links_in_a_loop_stop() {
    run_live(Live::any("live_links_in_a_loop_stop"), links_in_a_loop_stop);
}

async fn links_in_a_loop_stop(live: Live) {
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
    let fx = setup("loop", root).await;
    let team = live.team(vec![
        live.reader("solo", "Call the a_peek tool once, then stop.")
    ]);
    let (_, evs) = fx.run_to_end(&live, team).await;
    let steps = link_steps(&evs);
    assert_eq!(steps.len(), 65);
    let last = steps[64].payload["error"].as_str().unwrap();
    assert!(last.contains("64 calls in one step"), "{last}");
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
/// of the part, so it changed nothing.
#[test]
fn a_gate_holds_a_tool_back_until_the_document_allows_it() {
    run(async {
        let fx = setup("gated", gated_world("gated")).await;
        let (branch, ctx) = fx.live_branch().await;
        let refused = fx.mcp(&branch, "a_poke").await;
        assert_eq!(refused["isError"], true, "{refused}");
        assert_eq!(fx.mcp(&branch, "g_poke").await["isError"], false);
        let allowed = fx.mcp(&branch, "a_poke").await;
        assert_eq!(allowed["isError"], false, "{allowed}");
        assert_eq!(allowed["structuredContent"]["n"], 1, "{allowed}");
        let peek = fx.mcp(&branch, "a_peek").await;
        assert_eq!(
            peek["structuredContent"]["n"], 1,
            "the refused call changed nothing"
        );
        let evs = fx.state.store.events(&branch).await.unwrap();
        let first = &live::calls_named(&evs, "a_poke")[0].payload;
        assert!(first["part"].is_null(), "{first}");
        assert!(first["error"].as_str().unwrap().contains("/n"), "{first}");
        ctx.world.stop().await;
    })
}

/// A benchmark that scores itself gets the world's end state; a branch passes when every named
/// check does (here: the box was poked), and no gold world is made.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_scored_benchmark_checks_the_end_state() {
    run_live(
        Live::any("live_a_scored_benchmark_checks_the_end_state"),
        a_scored_benchmark_checks_the_end_state,
    );
}

/// A world of one toy part `box` whose benchmark scores itself: `poked` once it was poked.
fn scored_world() -> ToyRoot {
    let args = [
        "bench",
        "--integration",
        "toy-box",
        "--gold",
        "nope",
        "--scored",
    ];
    package_root(
        "scored",
        &[
            toy("toy-box", "integration", &["part"]),
            toy("toybench", "benchmark", &args),
        ],
    )
}

async fn a_scored_benchmark_checks_the_end_state(live: Live) {
    let root = scored_world();
    let fx = setup("scored", root).await;
    let team = live.team(vec![
        live.agent("solo", "Call the poke tool once, then stop.")
    ]);
    let (branch, evs) = fx.run_to_end(&live, team).await;
    let poked = live::calls_named(&evs, "poke")
        .iter()
        .any(|e| e.payload["error"].is_null());
    let status = fx.state.store.branch(&branch).await.unwrap().status;
    let (want, failed) = match poked {
        true => (BranchStatus::Passed, vec![]),
        false => (BranchStatus::Failed, vec!["poked".to_string()]),
    };
    assert_eq!(status, want);
    let c = live::check_of(&evs).unwrap();
    assert_eq!(c.differing_rows, Some(failed));
    assert_eq!(c.checks.unwrap()[0].name, "poked");
    assert_eq!(c.gold_db, None);
    assert!(c.final_saves.unwrap().contains_key("box"));
}

/// Two toy parts, `a` (its program run with `a_args`) and `b`, and a task whose setup makes
/// `setup`'s calls.
fn setup_world(tag: &str, a_args: &[&str], setup: &[Value], links: &[Value]) -> ToyRoot {
    let setup: Vec<String> = setup.iter().map(Value::to_string).collect();
    let links: Vec<String> = links.iter().map(Value::to_string).collect();
    let mut args = vec![
        "bench", "--scored", "--part", "a=toy-a", "--part", "b=toy-b",
    ];
    for s in &setup {
        args.extend(["--setup", s.as_str()]);
    }
    for l in &links {
        args.extend(["--link", l.as_str()]);
    }
    package_root(
        tag,
        &[
            toy("toy-a", "integration", a_args),
            toy("toy-b", "integration", &["part", "--prefix", "b_"]),
            toy("toybench", "benchmark", &args),
        ],
    )
}

/// A task's setup is made before any agent, as steps of the actor `setup` that save what they
/// changed; its tool is the world's own (agents don't get it), and the events it gives set off
/// links.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_tasks_setup_runs_before_the_agents() {
    run_live(
        Live::any("live_a_tasks_setup_runs_before_the_agents"),
        a_tasks_setup_runs_before_the_agents,
    );
}

/// The setup's poke, then the link it set off, both before the agent, then only the agent's
/// calls; the setup's step saved what it changed.
fn assert_setup_then_link_then_agent(evs: &[Event]) {
    let calls: Vec<&Event> = evs
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .collect();
    let actors: Vec<Option<&str>> = calls.iter().map(|e| e.agent.as_deref()).collect();
    assert_eq!(actors[..2], [Some("setup"), Some("link")]);
    assert!(actors[2..].iter().all(|a| *a == Some("solo")), "{actors:?}");
    let agent_start = live::first_of(evs, EventKind::AgentStart, "solo").unwrap();
    assert!(
        calls[1].seq < agent_start,
        "the setup came before the agent"
    );
    let (first, linked) = (&calls[0].payload, &calls[1].payload);
    let named = |p: &Value| (p["name"].clone(), p["part"].clone());
    assert_eq!(named(first), (json!("a_poke"), json!("a")));
    assert!(first["saves"]["a"].is_string(), "{first}");
    assert_eq!(named(linked), (json!("b_poke"), json!("b")));
}

async fn a_tasks_setup_runs_before_the_agents(live: Live) {
    let poke = json!({"part": "a", "tool": "a_poke"});
    let link = json!({"from": "a", "event": "rung", "to": "b", "tool": "b_poke"});
    let a_args = ["part", "--prefix", "a_", "--tool-event", "rung"];
    let fx = setup("setup", setup_world("setup", &a_args, &[poke], &[link])).await;
    assert_eq!(fx.agent_tools().await, ["a_peek", "b_peek"]);
    let team = live.team(vec![
        live.agent("solo", "Call the a_peek tool once, then stop.")
    ]);
    let (_, evs) = fx.run_to_end(&live, team).await;
    assert_setup_then_link_then_agent(&evs);
    for peek in live::calls_named(&evs, "a_peek") {
        assert_eq!(peek.payload["result"]["n"], 1, "{}", peek.payload);
    }
}

/// A setup call that fails ends the run's branch before any agent runs.
#[test]
fn a_failed_setup_ends_the_run() {
    run(async {
        let poke = json!({"part": "a", "tool": "a_poke"});
        let a_args = ["part", "--prefix", "a_", "--crash-on-call", "1"];
        let root = setup_world("setupfails", &a_args, &[poke], &[]);
        let fx = setup("setupfails", root).await;
        let e = runner::start_run(&fx.state, "t1", idle_team())
            .await
            .unwrap_err();
        assert!(
            format!("{e:#}").contains("setup call a_poke failed"),
            "{e:#}"
        );
        let runs = fx.state.store.runs().await.unwrap();
        let evs = fx
            .state
            .store
            .events(&runs[0].branches[0].id)
            .await
            .unwrap();
        assert!(
            evs.iter().all(|e| e.kind != EventKind::AgentStart),
            "no agent ran"
        );
    })
}

/// A world whose task's setup names a part or tool it doesn't have is refused.
#[test]
fn a_setup_of_nothing_is_refused() {
    run(async {
        let calls = [
            json!({"part": "nope", "tool": "a_poke"}),
            json!({"part": "b", "tool": "a_poke"}),
        ];
        for (i, call) in calls.into_iter().enumerate() {
            let root = setup_world(
                &format!("badsetup{i}"),
                &["part", "--prefix", "a_"],
                &[call],
                &[],
            );
            let hub = TestHub::new(std::slice::from_ref(&root.0));
            let bench = hub.hub.bench_describe("toybench").await.unwrap();
            assert!(plan(&hub.hub, &bench).await.is_err());
        }
    })
}
