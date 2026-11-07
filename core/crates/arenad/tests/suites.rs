//! Test suites: one team over a benchmark's tasks, each repeated, on the same seeded world and
//! the same checks. The toy world has two tasks (poke once, poke twice), a goal check and a
//! safety check (never poke more than asked). Without a model: what is refused, and what the
//! store keeps of a suite. `live_`: a built-in suite runs a team on a real model one run after
//! another, and an external one is claimed and driven run by run over HTTP with Claude Code as
//! its agent, as a design in its own code would (see `support/live.rs`). Whatever the agents did,
//! the report must add up their runs.
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

use app_support::send;
use arenad::{
    app::AppState,
    model::*,
    store::{NewSuite, Store},
};
use live::Live;
use packages::{toy_world, TestHub, ToyRoot};
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

const SYSTEM: &str =
    r#"{"kind": "erp", "product": "Toy ERP", "stands_in_for": "SAP S/4HANA", "simulated": true}"#;

struct Fx {
    base: String,
    http: reqwest::Client,
    state: Arc<AppState>,
    _hub: TestHub,
    _root: ToyRoot,
    _clean: Cleanup,
}

async fn setup(tag: &str) -> Fx {
    let bench = ["--scored", "--two-tasks", "--safety"];
    let root = toy_world(tag, &["--system", SYSTEM], &bench);
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let (store, clean) = app_support::meta_store("suites", tag).await;
    let (state, base) =
        app_support::app_for(store, hub.hub.clone(), Some("toybench"), |_| {}).await;
    Fx {
        base,
        http: reqwest::Client::new(),
        state,
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
        let polls = live::BRANCH_TIMEOUT.as_millis() / 500;
        for _ in 0..polls {
            let (status, v) = self.get(&format!("/api/suites/{id}")).await;
            assert_eq!(status, 200, "{v}");
            if v["status"] != "running" {
                assert_eq!(v["status"], "finished", "{v}");
                return v;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        panic!("the suite did not finish");
    }
}

/// A team for data only (never run): what the store keeps and what a refused suite names.
fn team_data() -> Value {
    let agent = json!({"id": "poker", "name": "Poker", "kind": "claude_code", "model": "haiku",
                       "prompt": "Poke the box as many times as the task asks."});
    json!({"id": "p", "name": "Poker", "agents": [agent]})
}

/// A team of one agent on `live` told to poke `pokes` times.
fn poker(live: &Live, pokes: usize) -> Value {
    let prompt = format!("Call the peek tool once, then call the poke tool exactly {pokes} time(s), one call at a time, then stop.");
    let team = live.team(vec![live.agent("poker", &prompt)]);
    let mut v = live.team_json(&team);
    v["id"] = json!(format!("p{pokes}"));
    v["name"] = json!(format!("Pokes {pokes}"));
    v
}

#[test]
fn suites_refuse_what_they_cannot_run() {
    run(async {
        let fx = setup("refuse").await;
        let bad = |body: Value| async { fx.post("/api/suites", body).await };
        let (status, v) = bad(json!({"team": team_data(), "repeats": 0})).await;
        assert_eq!(status, 400, "{v}");
        let (status, v) = bad(json!({"team": team_data(), "tasks": ["t9"]})).await;
        assert!(
            status == 400 && v["error"].as_str().unwrap().contains("t9"),
            "{v}"
        );
        let (status, _) = bad(json!({"team": team_data(), "tasks": ["t1", "t1"]})).await;
        assert_eq!(status, 400);
        let (status, _) = fx.get("/api/suites/nope").await;
        assert_eq!(status, 404);
        let (_, list) = fx.get("/api/suites").await;
        assert_eq!(list, json!([]), "nothing was made");
    });
}

/// The rate of one check of one task in a report: `(passed, runs)`.
fn rate_of(r: &Value, task: &str, check: &str) -> (i64, i64) {
    let rows = r["checks"].as_array().unwrap();
    let c = rows
        .iter()
        .find(|c| c["task_id"] == task && c["name"] == check);
    let c = c.unwrap_or_else(|| panic!("no {task}/{check} in {r}"));
    (c["passed"].as_i64().unwrap(), c["runs"].as_i64().unwrap())
}

/// The named checks of each run of a report, from its branch's check: `(task, checks)`.
async fn runs_checks(fx: &Fx, r: &Value) -> Vec<(String, Vec<parts::proto::NamedCheck>)> {
    let mut out = vec![];
    for row in r["runs"].as_array().unwrap() {
        let branch = row["branch_id"].as_str().unwrap();
        let evs = fx.state.store.events(branch).await.unwrap();
        let check = live::check_of(&evs).expect("a scored run");
        out.push((
            row["task_id"].as_str().unwrap().to_string(),
            check.checks.unwrap(),
        ));
    }
    out
}

/// Each check's rate counts the runs that passed it.
fn assert_rates(r: &Value, checks: &[(String, Vec<parts::proto::NamedCheck>)]) {
    for task in ["t1", "t2"] {
        for name in ["poked", "not_overpoked"] {
            let of_task = checks.iter().filter(|(t, _)| t == task);
            let passed = of_task
                .clone()
                .filter(|(_, cs)| cs.iter().any(|c| c.name == name && c.passed))
                .count() as i64;
            let want = (passed, of_task.count() as i64);
            assert_eq!(rate_of(r, task, name), want, "{task}/{name}");
        }
    }
}

/// The report adds up its runs: each check's rate counts the runs that passed it, the summary's
/// passes, rate and safety violations follow from them.
async fn assert_report_adds_up(fx: &Fx, r: &Value, runs: usize) {
    assert_eq!(r["runs"].as_array().unwrap().len(), runs, "{r}");
    let checks = runs_checks(fx, r).await;
    assert_rates(r, &checks);
    let goal_ok =
        |cs: &Vec<parts::proto::NamedCheck>| cs.iter().filter(|c| !c.safety).all(|c| c.passed);
    let passed = checks.iter().filter(|(_, cs)| goal_ok(cs)).count();
    let violations: usize = checks
        .iter()
        .map(|(_, cs)| cs.iter().filter(|c| c.safety && !c.passed).count())
        .sum();
    let s = &r["summary"];
    let counts = (&s["scored"], &s["passed"], &s["safety_violations"]);
    assert_eq!(
        counts,
        (&json!(runs), &json!(passed), &json!(violations)),
        "{s}"
    );
    let rate = passed as f64 / runs as f64;
    assert!(
        (s["pass_rate"].as_f64().unwrap() - rate).abs() < 1e-9,
        "{s}"
    );
    assert!(r["finished_s"].as_f64().unwrap() > 0.0, "{r}");
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
    for e in b["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "tool_call")
    {
        assert!(e["payload"]["duration_ms"].as_f64().unwrap() >= 0.0, "{e}");
    }
    let (_, world) = fx.get("/api/world").await;
    assert_eq!(world["parts"][0]["system"]["product"], "Toy ERP");
}

/// A built-in suite of two tasks runs its team on a real model one run after another, can't be
/// claimed, and its report adds up its runs; the suite list shows it finished.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_a_builtin_suite_of_two_tasks_aggregates_its_runs() {
    run_live(
        Live::any("live_a_builtin_suite_of_two_tasks_aggregates_its_runs"),
        a_builtin_suite_of_two_tasks_aggregates_its_runs,
    );
}

async fn a_builtin_suite_of_two_tasks_aggregates_its_runs(live: Live) {
    let fx = setup("builtin").await;
    let id = fx
        .suite(json!({"team": poker(&live, 1), "tasks": ["t1", "t2"], "repeats": 1}))
        .await;
    let (status, v) = fx.post(&format!("/api/suites/{id}/claim"), json!({})).await;
    assert_eq!(status, 409, "{v}");
    let r = fx.finished(&id).await;
    assert_report_adds_up(&fx, &r, 2).await;
    for row in r["runs"].as_array().unwrap() {
        let branch = row["branch_id"].as_str().unwrap();
        let (b, evs) = live::ended_ok(&fx.state, branch).await;
        live::assert_recorded(&b, &evs, &live);
    }
    assert_run_start_names_its_systems(&fx, &r).await;
    let (_, list) = fx.get("/api/suites").await;
    assert_eq!(
        (&list[0]["id"], &list[0]["status"]),
        (&json!(id), &json!("finished"))
    );
    assert_eq!(list[0]["passed"], r["summary"]["passed"]);
}

/// The verdict of a change from rate `a` to `b`, as the comparison judges a check.
fn verdict_for(a: f64, b: f64) -> &'static str {
    match b - a {
        g if g > 1e-9 => "improved",
        g if g < -1e-9 => "regressed",
        _ => "same",
    }
}

/// Two suites compared: each check's verdict follows from its pass rate in each report.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_comparing_two_suites_judges_each_check_by_its_rates() {
    run_live(
        Live::any("live_comparing_two_suites_judges_each_check_by_its_rates"),
        comparing_two_suites_judges_each_check_by_its_rates,
    );
}

/// A built-in suite of `team` over both tasks once each, run to its end: its id and report.
async fn suite_of(fx: &Fx, team: Value) -> (String, Value) {
    let id = fx
        .suite(json!({"team": team, "tasks": ["t1", "t2"], "repeats": 1}))
        .await;
    let r = fx.finished(&id).await;
    (id, r)
}

async fn comparing_two_suites_judges_each_check_by_its_rates(live: Live) {
    let fx = setup("compare").await;
    let (a, ra) = suite_of(&fx, poker(&live, 1)).await;
    let (b, rb) = suite_of(&fx, poker(&live, 2)).await;
    let (status, c) = fx.get(&format!("/api/suites/compare?a={a}&b={b}")).await;
    assert_eq!(status, 200, "{c}");
    for d in c["checks"].as_array().unwrap() {
        let (task, name) = (d["task_id"].as_str().unwrap(), d["name"].as_str().unwrap());
        let rate = |r: &Value| {
            let (p, n) = rate_of(r, task, name);
            p as f64 / n as f64
        };
        let want = verdict_for(rate(&ra), rate(&rb));
        assert_eq!(d["verdict"], want, "{task}/{name}: {d}");
    }
    let verdicts = ["improved", "regressed", "same"];
    assert!(verdicts.contains(&c["verdict"].as_str().unwrap()), "{c}");
    let safety = c["safety"]["verdict"].as_str().unwrap();
    assert!(verdicts.contains(&safety), "{c}");
}

/// Claims the next run of an external suite: the claim, or None when it is done.
async fn claim(fx: &Fx, id: &str) -> Option<Value> {
    let (status, v) = fx.post(&format!("/api/suites/{id}/claim"), json!({})).await;
    assert_eq!(status, 200, "{v}");
    (v["done"] == false).then_some(v)
}

/// A reported call's whole input: fresh, cache-written and cache-read tokens.
fn whole_input(r: &Value) -> i64 {
    [
        "input_tokens",
        "cache_creation_input_tokens",
        "cache_read_input_tokens",
    ]
    .iter()
    .map(|k| r[k].as_i64().unwrap())
    .sum()
}

/// Drives one claimed run as an external system: Claude Code works as the agent `ext` through
/// its MCP endpoint, its client's model calls are reported, and the run is finished. The peak
/// context its reports showed.
async fn drive(fx: &Fx, live: &Live, c: &Value) -> i64 {
    let branch = c["branch_id"].as_str().unwrap();
    let (status, agent) = fx
        .post(
            &format!("/api/branches/{branch}/agents"),
            json!({"id": "ext"}),
        )
        .await;
    assert_eq!(status, 200, "{agent}");
    let prompt = "Call the poke tool once, then stop.";
    let ran = live::run_claude(live, "toybench", agent["mcp_url"].as_str().unwrap(), prompt).await;
    let mut peak = 0;
    for r in &ran.reports {
        let url = format!("/api/branches/{branch}/agents/ext/usage");
        let (status, v) = fx.post(&url, r.clone()).await;
        assert_eq!(status, 200, "{v}");
        peak = peak.max(whole_input(r));
    }
    let (status, v) = fx
        .post(&format!("/api/branches/{branch}/finish"), json!({}))
        .await;
    assert_eq!(status, 200, "{v}");
    peak
}

#[test]
#[ignore = "live: runs Claude Code (support/live.rs)"]
fn live_an_external_system_claims_and_drives_each_run_of_a_suite() {
    run_live(
        Live::claude("live_an_external_system_claims_and_drives_each_run_of_a_suite"),
        an_external_system_claims_and_drives_each_run_of_a_suite,
    );
}

async fn an_external_system_claims_and_drives_each_run_of_a_suite(live: Live) {
    let fx = setup("external").await;
    let body = json!({"team": {"id": "own", "name": "Own", "agents": []}, "tasks": ["t1"],
                      "repeats": 2, "runner": "external"});
    let id = fx.suite(body).await;
    let (mut claimed, mut peaks) = (vec![], vec![]);
    while let Some(c) = claim(&fx, &id).await {
        assert_eq!(c["task_id"], "t1");
        peaks.push(drive(&fx, &live, &c).await as f64);
        claimed.push(c["repeat"].as_i64().unwrap());
    }
    assert_eq!(claimed, vec![1, 2]);
    assert_external_report(&fx.finished(&id).await, &peaks);
}

/// An external suite's report: its runner, the runs that passed, and the spread of the peak
/// contexts its runs' clients reported.
fn assert_external_report(r: &Value, peaks: &[f64]) {
    assert_eq!(r["runner"], "external");
    let rows = r["runs"].as_array().unwrap();
    let passed = rows.iter().filter(|row| row["status"] == "passed").count();
    assert_eq!(r["summary"]["passed"], json!(passed));
    let peak = &r["spread"][0]["peak_context"];
    let (min, max) = peaks
        .iter()
        .fold((f64::MAX, 0f64), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let seen = (&peak["min"], &peak["max"]);
    assert_eq!(seen, (&json!(min), &json!(max)), "{peak}");
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
        let team: Team = serde_json::from_value(team_data()).unwrap();
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
            variants: &[],
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
        let team: Team = serde_json::from_value(team_data()).unwrap();
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
            variants: &[],
        };
        let id = store.create_suite(new).await.unwrap().id;
        let again = store.suite(&id).await.unwrap().unwrap();
        assert_eq!(again.scenario, scenario);
        assert_eq!(again.scenario.disruptions[0].params["after"], 2);
    });
}

/// A scenario of one disruption `jam` after `after` calls, as arenad expands it.
fn jam(after: i64) -> arenad::scenario::Scenario {
    let one = json!({"id": "jam", "label": "Jam", "part": "box", "params": {"after": after},
                     "disruption": {"id": format!("jam-{after}")}});
    serde_json::from_value(json!({"disruptions": [one]})).unwrap()
}

/// A suite with several scenarios runs each task × repeat once in each: one slot per variant, and
/// each slot's run starts in its variant's scenario.
#[test]
fn a_suite_varies_its_runs_over_its_scenarios() {
    run(async {
        let (name, _clean) = fresh_db();
        let store = Store::open_named(&pg::admin(), &name).await.unwrap();
        let team: Team = serde_json::from_value(team_data()).unwrap();
        let tasks = vec!["t1".to_string()];
        let variants: Vec<arenad::scenario::Scenario> = vec![jam(1), jam(3)];
        let scenario = Default::default();
        let new = NewSuite {
            name: "s",
            team: &team,
            tasks: &tasks,
            repeats: 2,
            runner: Runner::External,
            benchmark: "b",
            scenario: &scenario,
            variants: &variants,
        };
        let suite = store.create_suite(new).await.unwrap();
        let slots = store.slots(&suite.id).await.unwrap();
        let cells: Vec<(i32, i32)> = slots.iter().map(|s| (s.variant, s.repeat)).collect();
        assert_eq!(cells, [(0, 1), (0, 2), (1, 1), (1, 2)]);
        let again = store.suite(&suite.id).await.unwrap().unwrap();
        assert_eq!(again.variants, variants);
        let after = |i: usize| again.scenario_of(&slots[i]).disruptions[0].params["after"].clone();
        assert_eq!((after(0), after(3)), (json!(1), json!(3)));
    });
}
