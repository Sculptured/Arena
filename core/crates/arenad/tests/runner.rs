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
    path::PathBuf,
    pin::Pin,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, AgentResult, AgentRun, Driver},
    app::{AppState, Config},
    exec::{BranchCtx, ToolExec},
    model::*,
    providers::ProviderConfig,
    runner::{self, StartPoint},
};
use fake_upstream::FakeUpstream;
use packages::{gold_action_args, TestHub};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("fake/model".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
    }
}

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

struct Fail;
impl Driver for Fail {
    fn run<'a>(&'a self, _: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async {
            AgentResult {
                ok: false,
                error: Some("boom".into()),
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
    state: Arc<AppState>,
    hub: TestHub,
    _clean: Cleanup,
}

/// Builds a driver from the state (drivers can only be installed once).
type MakeDriver = Box<dyn FnOnce(&Arc<AppState>) -> Arc<dyn Driver>>;

enum Drv {
    Open,
    Fail,
    Make(MakeDriver),
}

async fn setup(tag: &str, upstream: Option<&FakeUpstream>, failing: bool) -> Fx {
    let drv = if failing { Drv::Fail } else { Drv::Open };
    setup_full(tag, upstream, drv, &[], "retail").await
}

fn config(upstream: Option<&FakeUpstream>, base: String) -> Config {
    let mut config = Config::new();
    config.providers = ProviderConfig::default();
    if let Some(u) = upstream {
        config.providers.set_upstream(&u.base_url);
    }
    config.self_base = base;
    config
}

async fn setup_full(
    tag: &str,
    upstream: Option<&FakeUpstream>,
    drv: Drv,
    extra: &[PathBuf],
    bench: &str,
) -> Fx {
    let hub = TestHub::new(extra);
    let (store, clean) = app_support::meta_store("runner", tag).await;
    let (listener, base) = app_support::bind().await;
    let state = AppState::new(store, hub.hub.clone(), config(upstream, base));
    let state = Arc::new(state.with_benchmark(bench));
    let d: Arc<dyn Driver> = match drv {
        Drv::Open => Arc::new(OpenDriver::new(state.clone())),
        Drv::Fail => Arc::new(Fail),
        Drv::Make(f) => f(&state),
    };
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    app_support::serve(listener, &state);
    Fx {
        state,
        hub,
        _clean: clean,
    }
}

async fn finished(fx: &Fx, branch: &str) -> Branch {
    for _ in 0..600 {
        let b = fx.state.store.branch(branch).await.unwrap();
        if b.status != BranchStatus::Running {
            return b;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("branch still running");
}

fn tool_msg(calls: Vec<(&str, &str, Value)>) -> Value {
    let tcs: Vec<Value> = calls
        .into_iter()
        .map(|(id, name, args)| json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}))
        .collect();
    json!({"role": "assistant", "content": null, "tool_calls": tcs})
}

fn text_msg(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn triage_script() -> Vec<Value> {
    vec![
        tool_msg(vec![(
            "t1",
            "find_user_id_by_name_zip",
            json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}),
        )]),
        tool_msg(vec![(
            "t2",
            "get_order_details",
            json!({"order_id": "#W7181492"}),
        )]),
        tool_msg(vec![(
            "t3",
            "handoff",
            json!({"note": "Isabella wants to exchange an item in #W7181492."}),
        )]),
    ]
}

fn gold_write() -> Value {
    tool_msg(vec![(
        "r1",
        "exchange_delivered_order_items",
        gold_action_args("27", "exchange_delivered_order_items"),
    )])
}

fn wrong_write() -> Value {
    tool_msg(vec![(
        "r1",
        "modify_user_address",
        json!({"user_id": "isabella_johansson_2152", "address1": "1 Wrong St", "address2": "", "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"}),
    )])
}

fn kinds(evs: &[Event]) -> Vec<EventKind> {
    evs.iter()
        .map(|e| e.kind)
        .filter(|k| !matches!(k, EventKind::ModelCall | EventKind::ToolCall))
        .collect()
}

fn check_of(evs: &[Event]) -> CheckPayload {
    serde_json::from_value(evs.last().unwrap().payload.clone()).unwrap()
}

/// The resolver's input holds the ticket, the policy and the triage note.
fn assert_resolver_input(req: &Value) {
    let input = req["messages"][1]["content"].as_str().unwrap();
    assert!(input.contains("# Customer ticket") && input.contains("# Store policy"));
    assert!(
        input.contains("# Handoff note from triage")
            && input.contains("exchange an item in #W7181492")
    );
}

/// RunStart names the benchmark, its world, the seed, fingerprints and the base saves.
fn assert_run_start(rs: &Value) {
    assert_eq!(rs["task_id"], "27");
    assert_eq!(rs["benchmark"], "retail");
    assert_eq!(rs["world"][0]["integration"], "postgres");
    assert!(rs["seed"].is_u64(), "{rs}");
    assert!(rs["fingerprints"]["retail"].is_object(), "{rs}");
    assert_eq!(rs["saves"]["retail"], "base_retail_0_1_0_retail");
}

/// A passed check against the gold save, with the final state saved before the instances stop.
async fn assert_passed_check(fx: &Fx, check: &CheckPayload) {
    assert!(check.passed);
    assert_eq!(check.differing_rows, Some(vec![]));
    assert_eq!(
        check.gold_db.as_deref(),
        Some("gold_retail_0_1_0_27_retail")
    );
    let fin = check.final_saves.as_ref().expect("final saves");
    let key = "orders/#W7181492";
    let order = fx.hub.inspect_save(&fin["retail"], &[key]).await;
    assert_eq!(order[key]["doc"]["status"], "exchange requested");
}

#[test]
fn scripted_team_passes_task_27() {
    run(async {
        let mut script = triage_script();
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("pass", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::Handoff,
                EventKind::AgentEnd,
                EventKind::AgentStart,
                EventKind::AgentEnd,
                EventKind::Check
            ]
        );
        assert_eq!(evs[1].agent.as_deref(), Some("triage"));
        assert_run_start(&evs[0].payload);
        assert_passed_check(&fx, &check_of(&evs)).await;
        assert_resolver_input(&up.requests()[3]);
        assert!(fx.state.live.read().unwrap().is_empty());
    })
}

/// The step's arena overhead of kind `cat` (0 when it has none).
fn overhead(e: &Event, cat: &str) -> f64 {
    e.payload["overhead_ms"][cat].as_f64().unwrap_or(0.0)
}

/// Saves, recording, scoring and the world's start are timed on the steps they belong to, and
/// the metrics estimate production as the measured time minus them.
#[test]
fn arena_overhead_is_recorded_and_taken_out_of_the_estimate() {
    run(async {
        let mut script = triage_script();
        script.extend([gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("overhead", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert!(overhead(&evs[0], "restore") > 0.0, "{}", evs[0].payload);
        let write = evs
            .iter()
            .find(|e| e.kind == EventKind::ToolCall && e.payload["saves"].is_object());
        let write = write.expect("the write was saved");
        assert!(overhead(write, "save") > 0.0, "{}", write.payload);
        assert!(evs[1..].iter().all(|e| overhead(e, "record") > 0.0));
        assert!(overhead(evs.last().unwrap(), "score") > 0.0);
        let m = arenad::metrics::metrics(&b, &evs, arenad::prices::table(), chrono::Utc::now());
        assert!(m.overhead_recorded && m.overhead_s > 0.0);
        let est = m.estimated_s.unwrap();
        assert!((est - (m.measured_s - m.overhead_s).max(0.0)).abs() < 1e-9);
        assert_eq!(m.agents.len(), 2);
        // The fake upstream is a local server to arenad: five calls' tokens, no API charge.
        assert_eq!(
            (m.tokens, m.cost.label.as_str()),
            (75, arenad::metrics::NO_CHARGE)
        );
    })
}

#[test]
fn wrong_write_fails_with_differing_rows() {
    run(async {
        let mut script = triage_script();
        script.extend([wrong_write(), text_msg("Done.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("wrong", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error, None);
        let evs = fx.state.store.events(&branch).await.unwrap();
        let check = check_of(&evs);
        assert!(!check.passed);
        let rows = check.differing_rows.unwrap();
        assert!(
            rows.contains(&"users/isabella_johansson_2152".to_string()),
            "{rows:?}"
        );
        assert!(rows.contains(&"orders/#W7181492".to_string()), "{rows:?}");
        let mut sorted = rows.clone();
        sorted.sort();
        assert_eq!(rows, sorted);
    })
}

#[test]
fn driver_error_is_infra_error_not_failed() {
    run(async {
        let fx = setup("infra", None, true).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError);
        assert_eq!(b.error.as_deref(), Some("boom"));
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
    })
}

#[test]
fn model_cutoff_is_failed_not_infra_error() {
    run(async {
        let up = FakeUpstream::start(vec![
            json!({"role": "assistant", "content": "partial", "_finish_reason": "length"}),
        ])
        .await;
        let fx = setup("cutoff", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error.as_deref(), Some("model output was cut off"));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert!(!kinds(&evs).contains(&EventKind::Check));
    })
}

/// The second agent started without a note, and every agent was offered `message` and `inbox`.
fn assert_second_agent_has_no_note_and_talk_tools(up: &FakeUpstream) {
    let input = up.requests()[1]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(!input.contains("Handoff note"), "{input}");
    let tools = up.requests()[0]["tools"].to_string();
    assert!(
        tools.contains("\"message\"") && tools.contains("\"inbox\""),
        "{tools}"
    );
}

/// A new run never needs a handoff: an agent that leaves none ends normally, the next one
/// starts without a note, and the run's rules are recorded in `RunStart`.
#[test]
fn a_run_goes_on_without_handoffs() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("Nothing to hand off.")]).await;
        let fx = setup("nohandoff", Some(&up), false).await;
        let (_run, branch) = runner::start_run(&fx.state, "27", team()).await.unwrap();
        let b = finished(&fx, &branch).await;
        // Nothing was written, so the world differs from the gold one: failed, but not for a
        // missing handoff.
        assert_eq!((b.status, b.error), (BranchStatus::Failed, None));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![
                EventKind::RunStart,
                EventKind::AgentStart,
                EventKind::AgentEnd,
                EventKind::AgentStart,
                EventKind::AgentEnd,
                EventKind::Check
            ]
        );
        let rs = &evs[0].payload;
        assert_eq!(
            (&rs["runner"], &rs["handoff"]),
            (&json!("chain"), &json!("optional"))
        );
        assert_second_agent_has_no_note_and_talk_tools(&up);
    })
}

/// A branch under the old rules (a fork of a run recorded before runners) still fails when an
/// agent but the last leaves no handoff.
#[test]
fn a_legacy_branch_still_needs_its_handoff() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("I forgot to hand off.")]).await;
        let fx = setup("legacyhandoff", Some(&up), false).await;
        let (branch, ctx) = live_branch(&fx).await;
        assert_eq!(ctx.rules, Rules::legacy());
        runner::run_branch(fx.state.clone(), ctx, StartPoint::default()).await;
        let b = fx.state.store.branch(&branch).await.unwrap();
        assert_eq!(b.status, BranchStatus::Failed);
        assert_eq!(b.error.as_deref(), Some("no handoff"));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![EventKind::AgentStart, EventKind::AgentEnd]
        );
    })
}

/// Two agents that only finish once each has seen the other's message: possible only when
/// they run at once.
struct Pen {
    state: std::sync::Weak<AppState>,
}

impl Pen {
    async fn act(&self, exec: &ToolExec, run: &AgentRun) -> Result<(), String> {
        let (b, me) = (&run.branch, run.spec.id.as_str());
        let other = if me == "triage" { "resolver" } else { "triage" };
        let msg = json!({"to": other, "text": format!("hello from {me}")});
        exec.run_outcome(b, me, "message", msg, None).await?;
        for _ in 0..100 {
            let inbox = exec.run_outcome(b, me, "inbox", json!({}), None).await?;
            if inbox["messages"].as_array().is_some_and(|m| !m.is_empty()) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(format!("{me} never heard from {other}"))
    }
}

impl Driver for Pen {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let exec = ToolExec::new(self.state.upgrade().unwrap().store.clone());
            let error = self.act(&exec, &run).await.err();
            AgentResult {
                ok: error.is_none(),
                error,
                tokens: json!({}),
                cost_usd: None,
                cost_source: None,
                seconds: 0.0,
                failure: None,
            }
        })
    }
}

#[test]
fn the_concurrent_runner_runs_every_agent_at_once() {
    run(async {
        let make: MakeDriver = Box::new(|s| {
            Arc::new(Pen {
                state: Arc::downgrade(s),
            })
        });
        let fx = setup_full("concurrent", None, Drv::Make(make), &[], "retail").await;
        let started = runner::start_run_with(&fx.state, "27", team(), Runner::Concurrent);
        let (_run, branch) = started.await.unwrap();
        let b = finished(&fx, &branch).await;
        // Both agents finished (nothing was written, so the check fails).
        assert_eq!((b.status, b.error), (BranchStatus::Failed, None));
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(evs[0].payload["runner"], "concurrent");
        let k = kinds(&evs);
        assert_eq!(&k[1..3], [EventKind::AgentStart, EventKind::AgentStart]);
        let messages: Vec<_> = evs
            .iter()
            .filter(|e| e.kind == EventKind::Message)
            .collect();
        assert_eq!(messages.len(), 2);
        let inbox = evs.iter().rfind(|e| e.payload["name"] == "inbox").unwrap();
        let got = &inbox.payload["result"]["messages"][0];
        assert_eq!(
            (got["kind"].as_str(), got["to"].as_str()),
            (Some("message"), inbox.agent.as_deref())
        );
        assert_eq!(k.last(), Some(&EventKind::Check));
    })
}

/// A fresh branch of task 27 with its world started, made live as `spawn_branch` does before
/// it runs.
async fn live_branch(fx: &Fx) -> (String, Arc<BranchCtx>) {
    let t = team();
    let run_id = fx.state.store.create_run("27", &t).await.unwrap();
    let branch = fx
        .state
        .store
        .create_branch(&run_id, None, &t, &json!([]), None)
        .await
        .unwrap();
    let world = fx.hub.start_world("retail", &branch).await;
    let ctx = Arc::new(BranchCtx::new(branch.clone(), world, t, "27".into()));
    fx.state
        .live
        .write()
        .unwrap()
        .insert(branch.clone(), ctx.clone());
    (branch, ctx)
}

#[test]
fn run_branch_can_start_mid_chain_from_recorded_notes() {
    run(async {
        let up = FakeUpstream::start(vec![gold_write(), text_msg("Exchanged.")]).await;
        let fx = setup("mid", Some(&up), false).await;
        let (branch, ctx) = live_branch(&fx).await;
        let from = StartPoint {
            agent_index: 1,
            resume: None,
            notes: vec!["Carried note.".into()],
        };
        runner::run_branch(fx.state.clone(), ctx, from).await;
        let b = fx.state.store.branch(&branch).await.unwrap();
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert_eq!(
            kinds(&evs),
            vec![EventKind::AgentStart, EventKind::AgentEnd, EventKind::Check]
        );
        assert!(up.requests()[0]["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("Carried note."));
    })
}

// ---------------------------------------------------------------- worlds through the hub

const USER: &str = "isabella_johansson_2152";

/// Triage hands off; the resolver writes the address line in its prompt, waits for the other
/// branch's resolver to have written too, then reads.
struct TwoWriters {
    state: std::sync::Weak<AppState>,
    barrier: Arc<tokio::sync::Barrier>,
}

impl TwoWriters {
    async fn act(&self, exec: &ToolExec, run: &AgentRun) -> bool {
        let (b, me) = (&run.branch, run.spec.id.as_str());
        if me == "triage" {
            let note = json!({"note": "change the address"});
            return exec.run_outcome(b, me, "handoff", note, None).await.is_ok();
        }
        let args = json!({"user_id": USER, "address1": run.spec.prompt, "address2": "",
                          "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"});
        let wrote = exec
            .run_outcome(b, me, "modify_user_address", args, None)
            .await
            .is_ok();
        self.barrier.wait().await;
        let read = json!({"user_id": USER});
        wrote
            && exec
                .run_outcome(b, me, "get_user_details", read, None)
                .await
                .is_ok()
    }
}

impl Driver for TwoWriters {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(async move {
            let exec = ToolExec::new(self.state.upgrade().unwrap().store.clone());
            let ok = self.act(&exec, &run).await;
            let error = (!ok).then(|| "a tool call failed".to_string());
            AgentResult {
                ok,
                error,
                tokens: json!({}),
                cost_usd: None,
                cost_source: None,
                seconds: 0.0,
                failure: None,
            }
        })
    }
}

fn team_writing(line: &str) -> Team {
    let mut t = team();
    t.agents[1].prompt = line.to_string();
    t
}

/// The address line of `USER` in a save.
async fn line_in(fx: &Fx, save: &str) -> Value {
    let key = format!("users/{USER}");
    fx.hub.inspect_save(save, &[&key]).await[&key]["doc"]["address"]["address1"].clone()
}

/// The save the branch's address write made.
fn write_save(evs: &[Event]) -> String {
    let e = evs
        .iter()
        .find(|e| e.payload["name"] == "modify_user_address")
        .unwrap();
    e.payload["saves"]["retail"].as_str().unwrap().to_string()
}

/// What the branch's last user read returned.
fn line_read(evs: &[Event]) -> Value {
    let e = evs
        .iter()
        .rev()
        .find(|e| e.payload["name"] == "get_user_details")
        .unwrap();
    e.payload["result"]["address"]["address1"].clone()
}

/// A finished branch's events: what it read, wrote and saved last all show `line` only.
async fn assert_own_line(fx: &Fx, branch: &str, line: &str) -> (String, String) {
    let done = finished(fx, branch).await;
    assert_eq!(done.status, BranchStatus::Failed, "{:?}", done.error); // a wrong write, but finished
    let evs = fx.state.store.events(branch).await.unwrap();
    let fin = check_of(&evs).final_saves.unwrap()["retail"].clone();
    let save = write_save(&evs);
    assert_eq!(line_read(&evs), line);
    assert_eq!(line_in(fx, &fin).await, line);
    assert_eq!(line_in(fx, &save).await, line);
    (save, fin)
}

#[test]
fn two_branches_one_program() {
    run(async {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let make: MakeDriver = Box::new(move |state| {
            Arc::new(TwoWriters {
                state: Arc::downgrade(state),
                barrier,
            })
        });
        let fx = setup_full("two", None, Drv::Make(make), &[], "retail").await;
        // Both resolvers write before either reads (the barrier), on one postgres program.
        let (a, b) = tokio::join!(
            runner::start_run(&fx.state, "27", team_writing("1 Alpha Way")),
            runner::start_run(&fx.state, "27", team_writing("2 Beta Way")),
        );
        let ((_, a), (_, b)) = (a.unwrap(), b.unwrap());
        let (sa, fa) = assert_own_line(&fx, &a, "1 Alpha Way").await;
        let (sb, fb) = assert_own_line(&fx, &b, "2 Beta Way").await;
        assert!(sa != sb && fa != fb);
    })
}

fn poke() -> Value {
    tool_msg(vec![("p", "poke", json!({}))])
}

fn solo() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("solo")],
    }
}

/// The first poke ran and was saved; the second hit the crash.
fn assert_crash_events(evs: &[Event]) {
    let pokes: Vec<&Value> = evs
        .iter()
        .filter(|e| e.kind == EventKind::ToolCall)
        .map(|e| &e.payload)
        .collect();
    assert_eq!(pokes.len(), 2);
    assert!(
        pokes[0]["error"].is_null() && pokes[0]["saves"]["box"].is_string(),
        "{}",
        pokes[0]
    );
    assert!(pokes[1]["error"]
        .as_str()
        .unwrap()
        .contains("the box integration stopped"));
    assert!(!kinds(evs).contains(&EventKind::Check));
}

/// A toy world whose part crashes on its 2nd tool call, once; it logs every request.
fn crashy_world() -> (packages::ToyRoot, PathBuf) {
    let dir = packages::toy_dir("crashy");
    let (log, mark) = (dir.join("calls.log"), dir.join("crashed"));
    let (log_s, mark_s) = (
        log.to_string_lossy().into_owned(),
        mark.to_string_lossy().into_owned(),
    );
    let args = [
        "--crash-on-call",
        "2",
        "--crash-mark",
        &mark_s,
        "--log",
        &log_s,
    ];
    (packages::toy_world("crashy", &args, &[]), log)
}

#[test]
fn part_crash_ends_branch_infra_error() {
    run(async {
        let (root, log) = crashy_world();
        // Run 1 pokes twice (the 2nd crashes the part); run 2 pokes once and finishes.
        let up = FakeUpstream::start(vec![poke(), poke(), poke(), text_msg("Poked.")]).await;
        let fx = toy_fx("crash", Some(&up), &root, Drv::Open).await;
        let (_run, branch) = runner::start_run(&fx.state, "t1", solo()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError);
        let err = b.error.unwrap();
        assert!(
            err.starts_with("the box integration stopped: ")
                && err.contains("toy: crashing on call 2"),
            "{err}"
        );
        assert_crash_events(&fx.state.store.events(&branch).await.unwrap());
        // The agent stopped at the crash: no model call after the failed poke.
        assert_eq!(up.requests().len(), 2);
        // The crashed branch's instance was stopped (by the restarted program).
        let stop = format!("stop {}", arenad::world::instance_id(&branch, "box"));
        assert!(std::fs::read_to_string(&log).unwrap().contains(&stop));
        // The next run uses the restarted program and passes.
        let (_run, again) = runner::start_run(&fx.state, "t1", solo()).await.unwrap();
        let b = finished(&fx, &again).await;
        assert_eq!(b.status, BranchStatus::Passed, "{:?}", b.error);
    })
}

/// A test app whose benchmark is the toy world in `root`.
async fn toy_fx(tag: &str, up: Option<&FakeUpstream>, root: &packages::ToyRoot, drv: Drv) -> Fx {
    setup_full(tag, up, drv, std::slice::from_ref(&root.0), "toybench").await
}

/// The toy benchmark names no title for its instructions: the input uses the generic heading.
#[test]
fn instructions_without_a_title_get_the_generic_heading() {
    run(async {
        let root = packages::toy_world("title", &[], &[]);
        let up = FakeUpstream::start(vec![text_msg("Nothing to do.")]).await;
        let fx = toy_fx("title", Some(&up), &root, Drv::Open).await;
        let (_run, branch) = runner::start_run(&fx.state, "t1", solo()).await.unwrap();
        finished(&fx, &branch).await;
        let req = &up.requests()[0];
        let input = req["messages"][1]["content"].as_str().unwrap();
        assert!(
            input.contains("\n# Instructions\n\nBe kind to the box.\n"),
            "{input}"
        );
    })
}

#[test]
fn malformed_diff_is_infra_error_not_passed() {
    run(async {
        let root = packages::toy_world("baddiff", &["--bad-diff"], &[]);
        let up = FakeUpstream::start(vec![poke(), text_msg("Poked.")]).await;
        let fx = toy_fx("baddiff", Some(&up), &root, Drv::Open).await;
        let (_run, branch) = runner::start_run(&fx.state, "t1", solo()).await.unwrap();
        let b = finished(&fx, &branch).await;
        assert_eq!(b.status, BranchStatus::InfraError);
        let err = b.error.unwrap();
        assert!(
            err.contains("diff failed") && err.contains("bad result"),
            "{err}"
        );
        let evs = fx.state.store.events(&branch).await.unwrap();
        assert!(!kinds(&evs).contains(&EventKind::Check));
    })
}

#[test]
fn tool_name_clash_refused() {
    run(async {
        let root = packages::toy_world("clash", &[], &["--clash"]);
        let fx = toy_fx("clash", None, &root, Drv::Fail).await;
        let e = runner::start_run(&fx.state, "t1", solo())
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
