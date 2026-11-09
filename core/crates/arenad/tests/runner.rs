//! The runners: a team on a task, one agent after another or all at once, to the check. Without a
//! model: what is refused before a run, an agent whose provider can't be reached, and two
//! branches' worlds on one program. `live_`: teams on a real model (see `support/live.rs`).
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
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use app_support::send;
use arenad::{
    app::AppState,
    exec::BranchCtx,
    model::*,
    providers::ProviderConfig,
    runner::{self, StartPoint},
};
use live::Live;
use packages::TestHub;
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

struct Fx {
    state: Arc<AppState>,
    base: String,
    hub: TestHub,
    _clean: Cleanup,
}

/// An app with the real drivers whose benchmark is `bench` (on the packages plus `extra`);
/// `providers` replaces the environment's when given.
async fn setup(tag: &str, extra: &[PathBuf], bench: &str, providers: Option<ProviderConfig>) -> Fx {
    let hub = TestHub::new(extra);
    let (store, clean) = app_support::meta_store("runner", tag).await;
    let (state, base) = app_support::app_for(store, hub.hub.clone(), Some(bench), |c| {
        if let Some(p) = providers {
            c.providers = p;
        }
    })
    .await;
    Fx {
        state,
        base,
        hub,
        _clean: clean,
    }
}

/// A team on no reachable model: open agents on the local server nothing answers on.
fn unreachable_team() -> Team {
    let agent = |id: &str| AgentSpec {
        tools: Vec::new(),
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("m".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
        lead: false,
    };
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

/// The kinds of `evs` but model and tool calls.
fn kinds(evs: &[Event]) -> Vec<EventKind> {
    evs.iter()
        .map(|e| e.kind)
        .filter(|k| !matches!(k, EventKind::ModelCall | EventKind::ToolCall))
        .collect()
}

/// An agent whose provider can't be reached fails as an infrastructure problem: the branch ends
/// `infra_error` with the failure, after the agent's end, and no check.
#[test]
fn an_unreachable_provider_ends_the_branch_infra_error() {
    run(async {
        let providers = ProviderConfig::with_upstream(app_support::NOWHERE);
        let fx = setup("infra", &[], "retail", Some(providers)).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", unreachable_team())
            .await
            .unwrap();
        let b = live::ended(&fx.state, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError);
        let error = b.error.unwrap();
        assert!(error.starts_with("model call failed (502)"), "{error}");
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::AgentEnd
            ]
        );
        let end: AgentEndPayload =
            serde_json::from_value(evs.last().unwrap().payload.clone()).unwrap();
        assert!(!end.ok);
        assert!(fx.state.live.read().unwrap().is_empty());
    })
}

#[test]
fn tool_name_clash_refused() {
    run(async {
        let root = packages::toy_world("clash", &[], &["--clash"]);
        let fx = toy_fx("clash", &root, None).await;
        let e = runner::start_run(&fx.state, "t1", unreachable_team())
            .await
            .unwrap_err();
        assert!(
            format!("{e:#}").contains("two tools are called poke"),
            "{e:#}"
        );
        assert!(
            fx.state.store.runs().await.unwrap().is_empty(),
            "no run was recorded"
        );
    })
}

/// A fresh branch of `task` with its world started, made live as `spawn_branch` does before it
/// runs.
async fn live_branch(fx: &Fx, team: Team, bench: &str, task: &str) -> (String, Arc<BranchCtx>) {
    let run_id = fx.state.store.create_run(task, &team).await.unwrap();
    let branch = fx
        .state
        .store
        .create_branch(&run_id, None, &team, &json!([]), None)
        .await
        .unwrap();
    let world = fx.hub.start_world(bench, &branch).await;
    let ctx = Arc::new(BranchCtx::new(branch.clone(), world, team, task.into()));
    fx.state
        .live
        .write()
        .unwrap()
        .insert(branch.clone(), ctx.clone());
    (branch, ctx)
}

const USER: &str = "isabella_johansson_2152";

/// Arena's MCP endpoint for `agent` on `branch`: calls `name` with `args`, the tool's result.
async fn mcp(fx: &Fx, branch: &str, agent: &str, name: &str, args: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": name, "arguments": args}});
    let url = format!("{}/mcp/{branch}/{agent}", fx.base);
    let (status, v) = send(reqwest::Client::new().post(url).json(&body)).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["result"]["isError"], false, "{v}");
    v["result"]["structuredContent"].clone()
}

/// The address line of `USER` in a save.
async fn line_in(fx: &Fx, save: &str) -> Value {
    let key = format!("users/{USER}");
    fx.hub.inspect_save(save, &[&key]).await[&key]["doc"]["address"]["address1"].clone()
}

/// Writes `USER`'s address line `line` on `branch` through Arena's MCP endpoint.
async fn write_line(fx: &Fx, branch: &str, line: &str) -> Value {
    let args = json!({"user_id": USER, "address1": line, "address2": "",
                      "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"});
    mcp(fx, branch, "resolver", "modify_user_address", args).await
}

/// What `branch` reads and what its write saved show `line` only.
async fn assert_own_line(fx: &Fx, branch: &str, line: &str) {
    let user = json!({"user_id": USER});
    let read = mcp(fx, branch, "resolver", "get_user_details", user).await;
    assert_eq!(read["address"]["address1"], line, "{read}");
    let evs = fx.state.store.events(branch).await.unwrap();
    let w = live::calls_named(&evs, "modify_user_address")[0];
    let save = w.payload["saves"]["retail"].as_str().unwrap().to_string();
    assert_eq!(line_in(fx, &save).await, line);
}

/// Two branches' worlds run on one program and stay apart: each sees and saves its own write,
/// whichever wrote last.
#[test]
fn two_branches_on_one_program_keep_their_own_world() {
    run(async {
        let fx = setup("two", &[], "retail", None).await;
        let (a, actx) = live_branch(&fx, unreachable_team(), "retail", "27").await;
        let (b, bctx) = live_branch(&fx, unreachable_team(), "retail", "27").await;
        let (wa, wb) = tokio::join!(
            write_line(&fx, &a, "1 Alpha Way"),
            write_line(&fx, &b, "2 Beta Way"),
        );
        assert!(wa.is_object() && wb.is_object());
        for (branch, line) in [(&a, "1 Alpha Way"), (&b, "2 Beta Way")] {
            assert_own_line(&fx, branch, line).await;
        }
        actx.world.stop().await;
        bctx.world.stop().await;
    })
}

/// A test app whose benchmark is the toy world in `root`.
async fn toy_fx(tag: &str, root: &packages::ToyRoot, providers: Option<ProviderConfig>) -> Fx {
    setup(tag, std::slice::from_ref(&root.0), "toybench", providers).await
}

// ---------------------------------------------------------------- on a real model

/// RunStart names the benchmark, its world, the seed, fingerprints and the base saves.
fn assert_run_start(rs: &Value) {
    assert_eq!(rs["task_id"], "27");
    assert_eq!(rs["benchmark"], "retail");
    assert_eq!(rs["world"][0]["integration"], "postgres");
    assert!(rs["seed"].is_u64(), "{rs}");
    assert!(rs["fingerprints"]["retail"].is_object(), "{rs}");
    assert_eq!(rs["saves"]["retail"], "base_retail_0_1_0_retail");
    assert_eq!(
        (&rs["runner"], &rs["handoff"]),
        (&json!("chain"), &json!("optional"))
    );
}

/// The check of task 27, whatever the team did: against the gold save, passed exactly when no
/// row differs, the differing rows sorted, and the final state saved before the instances
/// stopped (an exchange in it when it passed).
async fn assert_scored(fx: &Fx, b: &Branch, check: &CheckPayload) {
    assert_eq!(
        check.gold_db.as_deref(),
        Some("gold_retail_0_1_0_27_retail")
    );
    let rows = check.differing_rows.clone().expect("differing rows");
    assert_eq!(check.passed, rows.is_empty(), "{rows:?}");
    assert_eq!(check.passed, b.status == BranchStatus::Passed);
    let mut sorted = rows.clone();
    sorted.sort();
    assert_eq!(rows, sorted);
    let fin = check.final_saves.as_ref().expect("final saves");
    let key = "orders/#W7181492";
    let order = fx.hub.inspect_save(&fin["retail"], &[key]).await;
    if check.passed {
        assert_eq!(order[key]["doc"]["status"], "exchange requested");
    } else {
        assert!(
            order[key]["doc"].is_object(),
            "the final save reads: {order}"
        );
    }
}

/// The step's arena overhead of kind `cat` (0 when it has none).
fn overhead(e: &Event, cat: &str) -> f64 {
    e.payload["overhead_ms"][cat].as_f64().unwrap_or(0.0)
}

/// Saves, recording, scoring and the world's start are timed on the steps they belong to, and
/// the metrics estimate production as the measured time minus them.
fn assert_overhead(b: &Branch, evs: &[Event]) {
    assert!(overhead(&evs[0], "restore") > 0.0, "{}", evs[0].payload);
    for w in evs
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall && e.payload["saves"].is_object())
    {
        assert!(overhead(w, "save") > 0.0, "{}", w.payload);
    }
    assert!(evs[1..].iter().all(|e| overhead(e, "record") > 0.0));
    assert!(overhead(evs.last().unwrap(), "score") > 0.0);
    let m = arenad::metrics::metrics(b, evs, arenad::prices::table(), chrono::Utc::now());
    assert!(m.overhead_recorded && m.overhead_s > 0.0);
    let est = m.estimated_s.unwrap();
    assert!((est - (m.measured_s - m.overhead_s).max(0.0)).abs() < 1e-9);
    assert_eq!(m.agents.len(), 2);
    assert!(m.tokens > 0, "the agents' tokens are counted");
}

/// With arenad's own loop: the resolver's input holds the ticket, the policy and any note the
/// triage left it.
fn assert_resolver_input(evs: &[Event]) {
    let Some(first) = evs
        .iter()
        .find(|e| e.kind == EventKind::ModelCall && e.agent.as_deref() == Some("resolver"))
    else {
        return;
    };
    let input = first.payload["request"]["messages"][1]["content"]
        .as_str()
        .unwrap();
    assert!(
        input.contains("# Customer ticket") && input.contains("# Store policy"),
        "{input}"
    );
    let note = evs
        .iter()
        .find(|e| e.kind == EventKind::Handoff && e.agent.as_deref() == Some("triage"));
    match note {
        Some(h) => {
            assert!(input.contains("# Handoff note from"), "{input}");
            assert!(
                input.contains(h.payload["note"].as_str().unwrap()),
                "{input}"
            );
        }
        None => assert!(!input.contains("Handoff note"), "{input}"),
    }
}

/// The retail benchmark's own team on task 27: the run starts, each agent runs once in order,
/// every step is recorded, the arena's overhead is timed, the check is computed against the
/// gold world, and the branch is no longer live.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_team_runs_task_27_and_arena_records_and_scores_it() {
    run_live(
        Live::any("live_a_team_runs_task_27_and_arena_records_and_scores_it"),
        a_team_runs_task_27_and_arena_records_and_scores_it,
    );
}

async fn a_team_runs_task_27_and_arena_records_and_scores_it(live: Live) {
    let fx = setup("live27", &[], "retail", None).await;
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    let team = live.bench_team(bench.team.unwrap());
    let (_run, branch) = runner::start_run(&fx.state, "27", team).await.unwrap();
    let (b, evs) = live::ended_ok(&fx.state, &branch).await;
    live::assert_recorded(&b, &evs, &live);
    assert_run_start(&evs[0].payload);
    let starts: Vec<Option<&str>> = evs
        .iter()
        .filter(|e| e.kind == EventKind::AgentStart)
        .map(|e| e.agent.as_deref())
        .collect();
    assert_eq!(
        starts,
        [Some("triage"), Some("resolver")],
        "one after another"
    );
    assert_eq!(evs.last().unwrap().kind, EventKind::Check, "{:?}", b.error);
    let check = live::check_of(&evs).unwrap();
    assert_scored(&fx, &b, &check).await;
    assert_overhead(&b, &evs);
    assert_resolver_input(&evs);
    assert!(fx.state.live.read().unwrap().is_empty());
}

/// A branch under the old rules (a fork of a run recorded before runners) fails when an agent
/// but the last ends without its handoff; one that hands off goes on to the next.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_legacy_branch_still_needs_its_handoff() {
    run_live(
        Live::any("live_a_legacy_branch_still_needs_its_handoff"),
        a_legacy_branch_still_needs_its_handoff,
    );
}

/// A triage and a resolver told to call no tool.
fn quiet_team(live: &Live) -> Team {
    let quiet = "Reply with one short sentence. Do not call any tool.";
    live.team(vec![
        live.agent("triage", quiet),
        live.agent("resolver", quiet),
    ])
}

async fn a_legacy_branch_still_needs_its_handoff(live: Live) {
    let fx = setup("legacyhandoff", &[], "retail", None).await;
    let (branch, ctx) = live_branch(&fx, quiet_team(&live), "retail", "27").await;
    assert_eq!(ctx.rules, Rules::legacy());
    runner::run_branch(fx.state.clone(), ctx, StartPoint::default()).await;
    let b = fx.state.store.branch(&branch).await.unwrap();
    let evs = fx.state.store.events(&branch).await.unwrap();
    let handed = evs.iter().any(|e| e.kind == EventKind::Handoff);
    let triage_ok = evs.iter().any(|e| {
        e.kind == EventKind::AgentEnd
            && e.agent.as_deref() == Some("triage")
            && e.payload["ok"] == true
    });
    if triage_ok && !handed {
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error.as_deref(), Some("no handoff"));
        assert_eq!(
            kinds(&evs),
            vec![EventKind::AgentStart, EventKind::AgentEnd]
        );
    } else if handed {
        eprintln!("note: the model handed off although told not to; the refusal was not exercised");
        assert!(evs
            .iter()
            .any(|e| e.kind == EventKind::AgentStart && e.agent.as_deref() == Some("resolver")));
    }
}

/// A new run never needs a handoff: an agent that leaves none ends normally, the next one starts
/// anyway, and every agent may message the others.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_run_goes_on_without_handoffs() {
    run_live(
        Live::any("live_a_run_goes_on_without_handoffs"),
        a_run_goes_on_without_handoffs,
    );
}

async fn a_run_goes_on_without_handoffs(live: Live) {
    let fx = setup("nohandoff", &[], "retail", None).await;
    let (_run, branch) = runner::start_run(&fx.state, "27", quiet_team(&live))
        .await
        .unwrap();
    let (b, evs) = live::ended_ok(&fx.state, &branch).await;
    live::assert_recorded(&b, &evs, &live);
    assert_ne!(b.error.as_deref(), Some("no handoff"));
    let starts = evs
        .iter()
        .filter(|e| e.kind == EventKind::AgentStart)
        .count();
    assert_eq!(starts, 2, "the resolver started after the triage");
    let rs = &evs[0].payload;
    let rules = (&rs["runner"], &rs["handoff"]);
    assert_eq!(rules, (&json!("chain"), &json!("optional")));
    if let Some(tools) = live::offered_tools(&evs, "triage") {
        assert!(
            tools.iter().any(|t| t == "message") && tools.iter().any(|t| t == "inbox"),
            "{tools:?}"
        );
    }
    assert_resolver_input(&evs);
}

/// The concurrent runner starts every agent before any ends; what one sends the other is
/// recorded as a message to it, and an inbox read after it holds it.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_the_concurrent_runner_runs_every_agent_at_once() {
    run_live(
        Live::any("live_the_concurrent_runner_runs_every_agent_at_once"),
        the_concurrent_runner_runs_every_agent_at_once,
    );
}

/// Each message went to the other agent, and an inbox read of it after holds it.
fn assert_messages_delivered(evs: &[Event]) {
    for m in evs.iter().filter(|e| e.kind == EventKind::Message) {
        let from = m.agent.as_deref().unwrap();
        let to = m.payload["to"].as_str().unwrap();
        let other = from != to && ["triage", "resolver"].contains(&to);
        assert!(other, "{}", m.payload);
        let read_after = evs.iter().find(|e| {
            e.seq > m.seq && e.agent.as_deref() == Some(to) && e.payload["name"] == "inbox"
        });
        if let Some(inbox) = read_after {
            let got = inbox.payload["result"]["messages"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let held = got.iter().any(|g| g["text"] == m.payload["text"]);
            assert!(held, "{}", inbox.payload);
        }
    }
}

async fn the_concurrent_runner_runs_every_agent_at_once(live: Live) {
    let fx = setup("concurrent", &[], "retail", None).await;
    let say = |other: &str| {
        format!("Call the message tool once with to=\"{other}\" and a one-line hello, then call inbox once, then stop.")
    };
    let team = live.team(vec![
        live.agent("triage", &say("resolver")),
        live.agent("resolver", &say("triage")),
    ]);
    let started = runner::start_run_with(&fx.state, "27", team, Runner::Concurrent);
    let (_run, branch) = started.await.unwrap();
    let (b, evs) = live::ended_ok(&fx.state, &branch).await;
    live::assert_recorded(&b, &evs, &live);
    assert_eq!(evs[0].payload["runner"], "concurrent");
    let k = kinds(&evs);
    assert_eq!(&k[1..3], [EventKind::AgentStart, EventKind::AgentStart]);
    assert_messages_delivered(&evs);
    assert_eq!(k.last(), Some(&EventKind::Check));
}

/// A branch can start mid-chain: the agent at the start point runs with the notes recorded
/// before it, and only it.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_run_branch_can_start_mid_chain_from_recorded_notes() {
    run_live(
        Live::any("live_run_branch_can_start_mid_chain_from_recorded_notes"),
        run_branch_can_start_mid_chain_from_recorded_notes,
    );
}

async fn run_branch_can_start_mid_chain_from_recorded_notes(live: Live) {
    let fx = setup("mid", &[], "retail", None).await;
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    let team = live.bench_team(bench.team.unwrap());
    let (branch, ctx) = live_branch(&fx, team, "retail", "27").await;
    let from = StartPoint {
        agent_index: 1,
        resume: None,
        notes: vec!["Carried note: the customer wants an exchange.".into()],
    };
    runner::run_branch(fx.state.clone(), ctx, from).await;
    let (_b, evs) = live::ended_ok(&fx.state, &branch).await;
    let k = kinds(&evs);
    assert_eq!(
        k,
        vec![EventKind::AgentStart, EventKind::AgentEnd, EventKind::Check]
    );
    assert!(evs.iter().all(|e| e.agent.as_deref() != Some("triage")));
    if let Some(first) = evs.iter().find(|e| e.kind == EventKind::ModelCall) {
        let input = first.payload["request"]["messages"][1]["content"]
            .as_str()
            .unwrap();
        assert!(input.contains("Carried note"), "{input}");
    }
}

/// A toy world whose part crashes on its first tool call, once; it logs every request.
fn crashy_world() -> (packages::ToyRoot, PathBuf) {
    let dir = packages::toy_dir("crashy");
    let (log, mark) = (dir.join("calls.log"), dir.join("crashed"));
    let (log_s, mark_s) = (
        log.to_string_lossy().into_owned(),
        mark.to_string_lossy().into_owned(),
    );
    let args = [
        "--crash-on-call",
        "1",
        "--crash-mark",
        &mark_s,
        "--log",
        &log_s,
    ];
    (packages::toy_world("crashy", &args, &[]), log)
}

/// A part that stops mid-run ends the branch `infra_error` with why, the agent stops there, the
/// crashed instance is stopped by the restarted program, and the next run uses that program.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_part_crash_ends_branch_infra_error() {
    run_live(
        Live::any("live_part_crash_ends_branch_infra_error"),
        part_crash_ends_branch_infra_error,
    );
}

/// The branch ended `infra_error` at the crash, the agent stopped there, and the crashed
/// instance was stopped (by the restarted program).
fn assert_crashed(b: &Branch, evs: &[Event], pokes: &[&Event], log: &std::path::Path) {
    assert_eq!(b.status, BranchStatus::InfraError);
    let err = b.error.clone().unwrap();
    let why =
        err.starts_with("the box integration stopped: ") && err.contains("toy: crashing on call 1");
    assert!(why, "{err}");
    let first = pokes[0].payload["error"].as_str().unwrap();
    assert!(first.contains("the box integration stopped"));
    assert_eq!(pokes.len(), 1, "the agent stopped at the crash");
    assert!(!kinds(evs).contains(&EventKind::Check));
    let stop = format!("stop {}", arenad::world::instance_id(&b.id, "box"));
    assert!(std::fs::read_to_string(log).unwrap().contains(&stop));
}

async fn part_crash_ends_branch_infra_error(live: Live) {
    let (root, log) = crashy_world();
    let fx = toy_fx("crash", &root, None).await;
    let solo = live.team(vec![
        live.agent("solo", "Call the poke tool once, then stop.")
    ]);
    let (_run, branch) = runner::start_run(&fx.state, "t1", solo.clone())
        .await
        .unwrap();
    let b = live::ended(&fx.state, &branch).await;
    let evs = fx.state.store.events(&branch).await.unwrap();
    let pokes = live::calls_named(&evs, "poke");
    if pokes.is_empty() {
        eprintln!("note: the model did not poke this time; the crash was not exercised");
        return;
    }
    assert_crashed(&b, &evs, &pokes, &log);
    let (_run, again) = runner::start_run(&fx.state, "t1", solo).await.unwrap();
    let b = live::ended(&fx.state, &again).await;
    assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
}

/// A part whose `diff` answers nonsense ends the branch `infra_error`, never passed, whatever the
/// agent did; the toy benchmark's instructions have no title, so the input uses the generic one.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_malformed_diff_is_infra_error_not_passed() {
    run_live(
        Live::any("live_malformed_diff_is_infra_error_not_passed"),
        malformed_diff_is_infra_error_not_passed,
    );
}

async fn malformed_diff_is_infra_error_not_passed(live: Live) {
    let root = packages::toy_world("baddiff", &["--bad-diff"], &[]);
    let fx = toy_fx("baddiff", &root, None).await;
    let solo = live.team(vec![
        live.agent("solo", "Call the poke tool once, then stop.")
    ]);
    let (_run, branch) = runner::start_run(&fx.state, "t1", solo).await.unwrap();
    let b = live::ended(&fx.state, &branch).await;
    assert_eq!(b.status, BranchStatus::InfraError);
    let err = b.error.unwrap();
    assert!(
        err.contains("diff failed") && err.contains("bad result"),
        "{err}"
    );
    let evs = fx.state.store.events(&branch).await.unwrap();
    assert!(!kinds(&evs).contains(&EventKind::Check));
    if let Some(first) = evs.iter().find(|e| e.kind == EventKind::ModelCall) {
        let input = first.payload["request"]["messages"][1]["content"]
            .as_str()
            .unwrap();
        assert!(
            input.contains("\n# Instructions\n\nBe kind to the box.\n"),
            "{input}"
        );
    }
}

/// A real Claude model whose output budget is one token: its answer is cut off, which is the
/// model's failure (the branch `failed`, no check), not the infrastructure's.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_cut_off_answer_fails_the_branch_not_infra_error() {
    run_live(
        Live::api("live_a_cut_off_answer_fails_the_branch_not_infra_error"),
        a_cut_off_answer_fails_the_branch_not_infra_error,
    );
}

async fn a_cut_off_answer_fails_the_branch_not_infra_error(live: Live) {
    if live.provider() != Some("anthropic") {
        eprintln!("skipped: needs ANTHROPIC_API_KEY (its output budget is set per call by arenad)");
        return;
    }
    let mut providers = arenad::app::Config::new().providers;
    providers.anthropic_max_tokens = 1;
    let fx = setup("cutoff", &[], "retail", Some(providers)).await;
    let team = live.team(vec![
        live.agent("solo", "Write a long answer about the order.")
    ]);
    let (_run, branch) = runner::start_run(&fx.state, "27", team).await.unwrap();
    let b = live::ended(&fx.state, &branch).await;
    assert_eq!(b.status, BranchStatus::Failed);
    assert_eq!(b.error.as_deref(), Some("model output was cut off"));
    let evs = fx.state.store.events(&branch).await.unwrap();
    assert!(!kinds(&evs).contains(&EventKind::Check));
}
