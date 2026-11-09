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

use arenad::{
    app::{AppState, Config},
    model::*,
    providers::ProviderConfig,
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

fn agent(id: &str) -> AgentSpec {
    AgentSpec {
        tools: Vec::new(),
        id: id.into(),
        name: id.into(),
        kind: AgentKind::Open,
        model: Some("m".into()),
        prompt: format!("You are {id}."),
        provider: None,
        read_only: false,
        lead: false,
        url: None,
        headers: Default::default(),
    }
}

fn team() -> Team {
    Team {
        id: "t".into(),
        name: "T".into(),
        agents: vec![agent("triage"), agent("resolver")],
    }
}

struct Fx {
    state: Arc<AppState>,
    _hub: TestHub,
    _clean: Cleanup,
    base: String,
    http: reqwest::Client,
}

/// An app that runs no agent (open mode offered on a server nothing answers on).
async fn setup(tag: &str) -> Fx {
    setup_with(tag, |_| {}).await
}

async fn setup_with(tag: &str, tweak: impl FnOnce(&mut Config)) -> Fx {
    setup_on(tag, TestHub::new(&[]), false, tweak).await
}

/// An app on the configured providers and Claude Code, for live tests.
async fn live_setup(tag: &str) -> Fx {
    setup_on(tag, TestHub::new(&[]), true, |_| {}).await
}

async fn setup_on(tag: &str, hub: TestHub, live: bool, tweak: impl FnOnce(&mut Config)) -> Fx {
    let (store, clean) = app_support::meta_store("api", tag).await;
    let (state, base) = match live {
        true => app_support::app_for(store, hub.hub.clone(), Some("retail"), tweak).await,
        false => app_support::idle_app(store, hub.hub.clone(), None, tweak).await,
    };
    Fx {
        state,
        _hub: hub,
        _clean: clean,
        base,
        http: reqwest::Client::new(),
    }
}

/// The test's `Cleanup` drops legacy branch and snapshot databases and the metadata database;
/// its `TestHub` drops the integration's instances and saves.
async fn teardown(fx: Fx, _branches: &[&str]) {
    drop(fx);
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        app_support::send(self.http.get(format!("{}{path}", self.base))).await
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let req = self.http.post(format!("{}{path}", self.base)).json(&body);
        app_support::send(req).await
    }

    async fn run_to_end(&self, task: &str, team: &Team) -> String {
        let (st, v) = self
            .post("/api/runs", json!({"task_id": task, "team": team}))
            .await;
        assert_eq!(st, 200, "{v}");
        let id = v["branch_id"].as_str().unwrap().to_string();
        self.wait(&id).await;
        id
    }

    /// The branch as the API shows it once it has ended (not for an infrastructure reason).
    async fn wait(&self, branch: &str) -> Value {
        live::ended_ok(&self.state, branch).await;
        self.get(&format!("/api/branches/{branch}")).await.1
    }
}

const ORDER: &str = "#W7181492";

/// Splits an SSE body into (event name, data) frames.
fn frames(body: &str) -> Vec<(String, String)> {
    body.split("\n\n")
        .filter_map(|f| {
            let mut name = None;
            let mut data = vec![];
            for l in f.lines() {
                if let Some(n) = l.strip_prefix("event:") {
                    name = Some(n.trim().to_string());
                } else if let Some(d) = l.strip_prefix("data:") {
                    data.push(d.trim_start().to_string());
                }
            }
            name.map(|n| (n, data.join("\n")))
        })
        .collect()
}

/// Reads the SSE stream until a `status` frame has arrived.
async fn read_stream(fx: &Fx, branch: &str) -> Vec<(String, String)> {
    let mut resp = fx
        .http
        .get(format!("{}/api/branches/{branch}/stream", fx.base))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    let mut body = String::new();
    let read = async {
        while let Some(chunk) = resp.chunk().await.unwrap() {
            body.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
            if frames(&body).iter().any(|(n, _)| n == "status") {
                break;
            }
        }
    };
    tokio::time::timeout(live::BRANCH_TIMEOUT, read)
        .await
        .expect("no status frame");
    frames(&body)
}

/// The health, task and default-team endpoints describe the retail benchmark and task 27.
async fn assert_health_and_tasks(fx: &Fx) {
    let (st, h) = fx.get("/api/health").await;
    assert_eq!(st, 200);
    assert!(h["open_available"].as_bool().unwrap());
    assert!(h["claude"].is_boolean() && h["codex"].is_boolean());
    assert_eq!(h["benchmark"]["name"], "retail", "{h}");
    assert_eq!(h["benchmark"]["world"][0]["integration"], "postgres", "{h}");
    let (_, tasks) = fx.get("/api/tasks").await;
    let t27 = tasks
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "27")
        .expect("task 27");
    assert!(
        t27["reason"].as_str().is_some_and(|r| !r.is_empty()),
        "{t27}"
    );
    let (_, dt) = fx.get("/api/team/default").await;
    assert_eq!(dt["agents"].as_array().unwrap().len(), 2);
}

/// Bad run requests: unknown task is 404; duplicate, malformed or no agents, or no team, is 400.
async fn assert_run_validation_errors(fx: &Fx) {
    let (st, v) = fx
        .post("/api/runs", json!({"task_id": "nope", "team": team()}))
        .await;
    assert_eq!((st, v["error"].is_string()), (404, true));
    for agents in [vec![agent("a"), agent("a")], vec![agent("Bad Id")], vec![]] {
        let t = Team { agents, ..team() };
        let (st, _) = fx
            .post("/api/runs", json!({"task_id": "27", "team": t}))
            .await;
        assert_eq!(st, 400);
    }
    assert_eq!(fx.post("/api/runs", json!({"task_id": "27"})).await.0, 400);
    assert_eq!(fx.get("/api/branches/nope").await.0, 404);
}

/// The branch's stream is ordered without gaps, ends with `check` and a `status` frame of the
/// branch's own status. Returns the number of frames and of events.
async fn assert_stream_ended(fx: &Fx, branch: &str) -> (usize, usize) {
    let fr = read_stream(fx, branch).await;
    let last = fr.last().unwrap();
    assert_eq!(last.0, "status");
    let status: Value = serde_json::from_str(&last.1).unwrap();
    let b = fx.state.store.branch(branch).await.unwrap();
    assert_eq!(status["status"], json!(b.status), "{status}");
    let evs: Vec<Value> = fr
        .iter()
        .filter(|(n, _)| n == "event")
        .map(|(_, d)| serde_json::from_str(d).unwrap())
        .collect();
    let seqs: Vec<i64> = evs.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
    assert_eq!(
        seqs,
        (1..=seqs.len() as i64).collect::<Vec<_>>(),
        "ordered, no gaps or duplicates"
    );
    assert_eq!(evs.last().unwrap()["kind"], "check");
    (fr.len(), seqs.len())
}

/// The branch endpoint has all `n_events` and the run list shows the branch with its status.
async fn assert_branch_and_run_listed(fx: &Fx, branch: &str, run_id: &str, n_events: usize) {
    let (_, b) = fx.get(&format!("/api/branches/{branch}")).await;
    assert_eq!(b["events"].as_array().unwrap().len(), n_events);
    let (_, runs) = fx.get("/api/runs").await;
    let r = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run_id)
        .unwrap();
    assert_eq!(r["branches"][0]["id"], branch);
    assert_eq!(r["branches"][0]["status"], b["status"]);
}

#[test]
fn health_tasks_and_run_validation() {
    run(async {
        let fx = setup("health").await;
        assert_health_and_tasks(&fx).await;
        assert_run_validation_errors(&fx).await;
        assert!(fx.state.store.runs().await.unwrap().is_empty());
    })
}

/// A run started over HTTP streams its events in order to its end, then its status; a stream
/// opened after the end replays it all; the branch and the run list agree.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_post_run_then_stream_until_status() {
    run_live(
        Live::any("live_post_run_then_stream_until_status"),
        post_run_then_stream_until_status,
    );
}

async fn post_run_then_stream_until_status(live: Live) {
    let fx = live_setup("stream").await;
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    let team = live.bench_team(bench.team.unwrap());
    let (st, v) = fx
        .post("/api/runs", json!({"task_id": "27", "team": team}))
        .await;
    assert_eq!(st, 200, "{v}");
    let branch = v["branch_id"].as_str().unwrap().to_string();
    let run_id = v["run_id"].as_str().unwrap().to_string();
    let (frames, seqs) = assert_stream_ended(&fx, &branch).await;
    let (b, evs) = live::ended_ok(&fx.state, &branch).await;
    live::assert_recorded(&b, &evs, &live);
    let again = read_stream(&fx, &branch).await;
    assert_eq!(again.len(), frames);
    assert_branch_and_run_listed(&fx, &branch, &run_id, seqs).await;
    teardown(fx, &[&branch]).await;
}

fn solo(live: &Live) -> Team {
    let prompt = "You are a retail support agent. Read the order with get_order_details, then do what the ticket asks.";
    live.team(vec![live.agent("resolver", prompt)])
}

/// A branch compared with itself has no divergence and no differing rows; an unknown branch is
/// 404 and a missing side 400.
async fn compare_edge_cases(fx: &Fx, a: &str) {
    let (_, same) = fx.get(&format!("/api/compare?a={a}&b={a}")).await;
    assert!(same["first_divergent_seq"].is_null());
    assert!(same["differing_rows"].as_array().unwrap().is_empty());
    assert_eq!(fx.get(&format!("/api/compare?a={a}&b=nope")).await.0, 404);
    assert_eq!(fx.get(&format!("/api/compare?a={a}")).await.0, 400);
}

/// The final save of part `retail` of a finished branch.
async fn final_save(fx: &Fx, branch: &str) -> String {
    let evs = fx.state.store.events(branch).await.unwrap();
    let check = live::check_of(&evs).expect("a check");
    check.final_saves.unwrap()["retail"].clone()
}

/// Two runs compared: their statuses and tokens, and differing rows that are exactly what the
/// part's `diff` of their final saves finds, each side's doc read from its save (keys without a
/// part prefix in a one-part world).
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_compare_diffs_the_final_saves_of_two_runs() {
    run_live(
        Live::any("live_compare_diffs_the_final_saves_of_two_runs"),
        compare_diffs_the_final_saves_of_two_runs,
    );
}

/// The differing rows are exactly the part's `diff` of the two final saves, each side's doc read
/// from its save.
async fn assert_rows_are_the_parts_diff(fx: &Fx, v: &Value, a: &str, b: &str) {
    let (sa, sb) = (final_save(fx, a).await, final_save(fx, b).await);
    let mut want = fx._hub.diff(json!({"save": sa}), json!({"save": sb})).await;
    want.sort();
    let rows = v["differing_rows"].as_array().unwrap();
    let mut keys: Vec<String> = rows
        .iter()
        .map(|r| r["key"].as_str().unwrap().to_string())
        .collect();
    keys.sort();
    assert_eq!(keys, want);
    for r in rows {
        let key = r["key"].as_str().unwrap();
        let doc_a = &fx._hub.inspect_save(&sa, &[key]).await[key];
        let doc_b = &fx._hub.inspect_save(&sb, &[key]).await[key];
        assert_eq!((&r["a"], &r["b"]), (doc_a, doc_b), "{key}");
    }
}

async fn compare_diffs_the_final_saves_of_two_runs(live: Live) {
    let fx = live_setup("compare").await;
    let a = fx.run_to_end("27", &solo(&live)).await;
    let b = fx.run_to_end("27", &solo(&live)).await;
    let (st, v) = fx.get(&format!("/api/compare?a={a}&b={b}")).await;
    assert_eq!(st, 200, "{v}");
    for (side, id) in [("a", &a), ("b", &b)] {
        let br = fx.state.store.branch(id).await.unwrap();
        assert_eq!(v[side]["status"], json!(br.status), "{v}");
        assert!(v[side]["tokens"].as_i64().unwrap() > 0, "{v}");
        assert!(v[side]["seconds"].as_f64().unwrap() >= 0.0, "{v}");
    }
    assert_rows_are_the_parts_diff(&fx, &v, &a, &b).await;
    let first = &v["first_divergent_seq"];
    assert!(first.is_null() || first.as_i64().unwrap() >= 1, "{first}");
    compare_edge_cases(&fx, &a).await;
    teardown(fx, &[&a, &b]).await;
}

/// The rows request for `keys` at step `seq` of branch `b`, plus `extra` query text.
fn rows_path(b: &str, seq: i64, keys: &str, extra: &str) -> String {
    format!("/api/branches/{b}/rows?seq={seq}&keys={keys}{extra}")
}

/// Unknown part, malformed key, too many keys and unknown step are 400; unknown branch 404.
async fn bad_rows_requests(fx: &Fx, b: &str, seq: i64) {
    let many: Vec<String> = (0..51).map(|i| format!("users/u{i}")).collect();
    for (path, want) in [
        (rows_path(b, seq, "users/x", "&part=nope"), 400),
        (rows_path(b, seq, "nokey", ""), 400),
        (rows_path(b, seq, &many.join(","), ""), 400),
        (rows_path(b, 9999, "users/x", ""), 400),
        (rows_path("nope", 1, "users/x", ""), 404),
    ] {
        assert_eq!(fx.get(&path).await.0, want, "{path}");
    }
}

/// Rows are read with `inspect` from the save holding the part's state after the step: at the
/// run's start the base save, and after each step that saved, that save, whatever the agent
/// wrote; a key the part doesn't have is null.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_rows_read_each_steps_save() {
    run_live(
        Live::any("live_rows_read_each_steps_save"),
        rows_read_each_steps_save,
    );
}

/// The rows of what step `e` wrote, read at it, are what its save holds.
async fn assert_rows_read_the_save(fx: &Fx, b: &str, e: &Event) {
    let save = e.payload["saves"]["retail"].as_str().unwrap();
    let written: Vec<String> =
        serde_json::from_value(e.payload["writes"].clone()).unwrap_or_default();
    for key in written.iter().filter(|k| k.contains('/')) {
        let q = key.replace('#', "%23");
        let (st, v) = fx.get(&rows_path(b, e.seq, &q, "&part=retail")).await;
        assert_eq!(st, 200, "{v}");
        let saved = fx._hub.inspect_save(save, &[key]).await;
        assert_eq!(v["rows"][key], saved[key], "{key} at {}", e.seq);
    }
}

async fn rows_read_each_steps_save(live: Live) {
    let fx = live_setup("rows").await;
    let b = fx.run_to_end("27", &solo(&live)).await;
    let evs = fx.state.store.events(&b).await.unwrap();
    let order = format!("orders/{ORDER}");
    let keys = "orders/%23W7181492,users/nobody";
    let (st, v) = fx.get(&rows_path(&b, 1, keys, "")).await;
    assert_eq!((st, &v["seq"]), (200, &json!(1)), "{v}");
    let base = fx
        ._hub
        .inspect_save("base_retail_0_1_0_retail", &[&order])
        .await;
    assert_eq!(v["rows"][&order], base[&order], "{v}");
    assert!(v["rows"]["users/nobody"].is_null());
    for e in evs
        .iter()
        .filter(|e| e.payload["saves"]["retail"].is_string())
    {
        assert_rows_read_the_save(&fx, &b, e).await;
    }
    bad_rows_requests(&fx, &b, 1).await;
    teardown(fx, &[&b]).await;
}

/// A bad step, an unknown agent in the changes and a malformed body are 400; an unknown branch
/// is 404.
async fn assert_fork_errors(fx: &Fx, url: &str, at: i64) {
    assert_eq!(fx.post(url, json!({"at_seq": 9999})).await.0, 400);
    assert_eq!(
        fx.post(
            url,
            json!({"at_seq": at, "changes": {"ghost": {"prompt": "x"}}})
        )
        .await
        .0,
        400
    );
    assert_eq!(fx.post(url, json!({"nope": 1})).await.0, 400);
    assert_eq!(
        fx.post("/api/branches/nope/fork", json!({"at_seq": 1}))
            .await
            .0,
        404
    );
}

/// A fork over HTTP: bad requests are mapped to 400 and 404, and a good one starts a child of
/// the branch in the same run that runs to its end.
#[test]
#[ignore = "live: runs a real model (support/live.rs)"]
fn live_fork_over_http_maps_errors_and_starts_a_branch() {
    run_live(
        Live::any("live_fork_over_http_maps_errors_and_starts_a_branch"),
        fork_over_http_maps_errors_and_starts_a_branch,
    );
}

async fn fork_over_http_maps_errors_and_starts_a_branch(live: Live) {
    let fx = live_setup("fork").await;
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    let team = live.bench_team(bench.team.unwrap());
    let b = fx.run_to_end("27", &team).await;
    let evs = fx.state.store.events(&b).await.unwrap();
    let at = live::first_of(&evs, EventKind::AgentStart, "resolver").unwrap();
    let url = format!("/api/branches/{b}/fork");
    assert_fork_errors(&fx, &url, at).await;
    let (st, v) = fx.post(&url, json!({"at_seq": at})).await;
    assert_eq!(st, 200, "{v}");
    let child = v["branch_id"].as_str().unwrap().to_string();
    assert_eq!(v["start_seq"], at, "{v}");
    let done = fx.wait(&child).await;
    assert_eq!(done["parent_id"], b.as_str());
    let parent = fx.state.store.branch(&b).await.unwrap();
    let c = fx.state.store.branch(&child).await.unwrap();
    assert_eq!(c.run_id, parent.run_id);
    let cevs = fx.state.store.events(&child).await.unwrap();
    live::assert_recorded(&c, &cevs, &live);
    teardown(fx, &[&b, &child]).await;
}

#[test]
fn open_mode_is_rejected_without_a_key() {
    run(async {
        let fx = setup_with("nokey", |c| {
            c.providers = ProviderConfig::default();
        })
        .await;
        let (_, h) = fx.get("/api/health").await;
        assert_eq!(h["open_available"], false);
        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_eq!(
            (st, v["error"].as_str()),
            (400, Some("open mode needs OPENROUTER_API_KEY"))
        );
        // A fork's changes are judged on the team it would run, so an unknown branch is 404
        // first (tests/providers.rs checks the refusals of a fork).
        let (st, _) = fx
            .post(
                "/api/branches/whatever/fork",
                json!({"at_seq": 1, "changes": {"resolver": {"kind": "open"}}}),
            )
            .await;
        assert_eq!(st, 404);
        assert!(
            fx.state.store.runs().await.unwrap().is_empty(),
            "no run was created"
        );
        drop(fx.state);
    })
}

#[test]
fn stream_ends_when_shutdown_fires() {
    run(async {
        // A branch that stays running: a run row and a running branch, no agent.
        let fx = setup("shutdown").await;
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let b = fx
            .state
            .store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
        let mut resp = fx
            .http
            .get(format!("{}/api/branches/{b}/stream", fx.base))
            .send()
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        fx.state.shutdown.send(true).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), async {
            while resp.chunk().await.unwrap().is_some() {}
        })
        .await;
        assert!(ended.is_ok(), "stream did not end on shutdown");
        drop(fx.state);
    })
}

#[test]
fn compare_of_branch_without_final_state_is_409() {
    run(async {
        let fx = setup("nodb").await;
        let t = team();
        let run_id = fx.state.store.create_run("27", &t).await.unwrap();
        let ghost = fx
            .state
            .store
            .create_branch(&run_id, None, &t, &json!([]), None)
            .await
            .unwrap();
        fx.state
            .store
            .set_status(
                &ghost,
                BranchStatus::InfraError,
                Some("setup failed".into()),
            )
            .await
            .unwrap();
        let (st, v) = fx.get(&format!("/api/compare?a={ghost}&b={ghost}")).await;
        assert_eq!(st, 409, "{v}");
        assert_eq!(v["error"], "this branch has no final state yet");
        drop(fx.state);
    })
}

#[test]
fn api_rejects_foreign_host_headers() {
    run(async {
        let fx = setup("host").await;
        let port = fx.base.rsplit(':').next().unwrap().to_string();
        for (host, want) in [
            ("evil.example", 403),
            (&format!("evil.example:{port}"), 403),
            ("127.0.0.1.evil.example", 403),
            ("127.0.0.1", 200),
            (&format!("127.0.0.1:{port}"), 200),
            (&format!("localhost:{port}"), 200),
            (&format!("[::1]:{port}"), 200),
        ] {
            let r = fx
                .http
                .get(format!("{}/api/health", fx.base))
                .header("Host", host)
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), want, "host {host}");
        }
        drop(fx.state);
    })
}

// ---------------------------------------------------------------- hand-made toy worlds

/// An app whose hub also has the toy integration `toy-<tag>`, for branches recorded by hand (no
/// agent ran them: their events are written to the store as data, to read back).
struct Toy {
    fx: Fx,
    part: String,
    _root: packages::ToyRoot,
}

async fn toy(tag: &str) -> Toy {
    toy_with(tag, &[]).await
}

/// `toy` with the toy integration started with `part_args`.
async fn toy_with(tag: &str, part_args: &[&str]) -> Toy {
    let root = packages::toy_world(tag, part_args, &[]);
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let fx = setup_on(tag, hub, false, |_| {}).await;
    let part = format!("toy-{tag}");
    Toy {
        fx,
        part,
        _root: root,
    }
}

impl Toy {
    /// A toy save `save` whose counter is `n`.
    async fn save(&self, save: &str, n: usize) {
        let c = self.fx.state.hub.client(&self.part).await.unwrap();
        let start = json!({"instance": "mk", "config": {}, "seed": 0, "clock_ms": 0});
        c.call::<Value>("start", start).await.unwrap();
        let poke = json!({"instance": "mk", "name": "poke", "args": {}, "clock_ms": 0});
        for _ in 0..n {
            c.call::<Value>("call_tool", poke.clone()).await.unwrap();
        }
        let dir = self.fx.state.hub.save_dir(&self.part, save);
        let params = json!({"instance": "mk", "save": save, "dir": dir});
        c.call::<Value>("save", params).await.unwrap();
    }

    /// A finished branch: a `RunStart` of a world of toy `parts` starting from `base`, then
    /// `events`. Its id.
    async fn branch(&self, parts: &[&str], base: Value, events: Vec<(EventKind, Value)>) -> String {
        let store = &self.fx.state.store;
        let run_id = store.create_run("t1", &team()).await.unwrap();
        let b = store
            .create_branch(&run_id, None, &team(), &json!([]), None)
            .await
            .unwrap();
        let world: Vec<Value> = parts
            .iter()
            .map(|p| json!({"part": p, "integration": self.part, "config": {}}))
            .collect();
        let start = json!({"benchmark": "toybench", "world": world, "seed": 0, "saves": base});
        store
            .append(&b, None, EventKind::RunStart, start)
            .await
            .unwrap();
        for (kind, payload) in events {
            store.append(&b, None, kind, payload).await.unwrap();
        }
        store
            .set_status(&b, BranchStatus::Passed, None)
            .await
            .unwrap();
        b
    }
}

fn poked(save: &str) -> (EventKind, Value) {
    let p = json!({"name": "poke", "error": null, "writes": ["box/n"], "part": "box", "saves": {"box": save}});
    (EventKind::ToolCall, p)
}

/// The part's refusal of a key (`-32602`) is a 400 with its message; a save that is gone a 409.
#[test]
fn rows_part_errors_are_400_and_409() {
    run(async {
        let t = toy("rowserr").await;
        t.save("s_t1", 1).await;
        let b = t
            .branch(
                &["box"],
                json!({"box": "s_t1"}),
                vec![poked("s_t1"), poked("s_gone")],
            )
            .await;
        let (st, v) = t.fx.get(&rows_path(&b, 2, "box/n", "")).await;
        assert_eq!((st, &v["rows"]["box/n"]["n"]), (200, &json!(1)), "{v}");
        let (st, v) = t.fx.get(&rows_path(&b, 2, "bad/x", "")).await;
        assert_eq!(
            (st, v["error"].as_str()),
            (400, Some("the toy has no bad keys"))
        );
        let (st, v) = t.fx.get(&rows_path(&b, 3, "box/n", "")).await;
        assert_eq!(
            (st, v["error"].as_str()),
            (409, Some("the saved state for that step is gone"))
        );
    })
}

fn checked(final_saves: Value) -> (EventKind, Value) {
    (
        EventKind::Check,
        json!({"passed": true, "differing_rows": [], "final_saves": final_saves}),
    )
}

/// In a world of two parts, differing keys are prefixed `part:`; a branch without a final save
/// of a part can't be compared (409).
#[test]
fn compare_prefixes_keys_in_a_two_part_world() {
    run(async {
        let t = toy("twoparts").await;
        for (save, n) in [("s_a1", 1), ("s_a2", 2), ("s_b", 0)] {
            t.save(save, n).await;
        }
        let base = json!({"a": "s_b", "b": "s_b"});
        let branch = async |end: Value| {
            t.branch(&["a", "b"], base.clone(), vec![checked(end)])
                .await
        };
        let x = branch(json!({"a": "s_a1", "b": "s_b"})).await;
        let y = branch(json!({"a": "s_a2", "b": "s_b"})).await;
        let z = branch(json!({"a": "s_a1"})).await;
        let (st, v) = t.fx.get(&format!("/api/compare?a={x}&b={y}")).await;
        assert_eq!(st, 200, "{v}");
        let want = json!([{"key": "a:box/n", "a": {"n": 1}, "b": {"n": 2}}]);
        assert_eq!(v["differing_rows"], want);
        let (st, v) = t.fx.get(&format!("/api/compare?a={x}&b={z}")).await;
        assert_eq!(st, 409, "{v}");
    })
}

/// Deleting a run drops each save its events name straight away (no probe first); a save the
/// part says is already gone (-32002) is no error and doesn't count as dropped.
#[test]
fn delete_treats_an_unknown_save_as_gone() {
    run(async {
        let logs = packages::ToyRoot(packages::toy_dir("dellog"));
        std::fs::create_dir_all(&logs.0).unwrap();
        let log = logs.0.join("calls.log");
        let t = toy_with("delgone", &["--log", &log.to_string_lossy()]).await;
        t.save("s_d1", 1).await;
        let events = vec![poked("s_d1"), poked("s_dgone")];
        let b = t.branch(&["box"], json!({"box": "s_d1"}), events).await;
        let run_id = t.fx.state.store.branch(&b).await.unwrap().run_id;
        let url = format!("{}/api/runs/{run_id}", t.fx.base);
        let (st, v) = app_support::send(t.fx.http.delete(url)).await;
        assert_eq!((st, &v["dropped"]), (200, &json!(1)), "{v}");
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().filter(|l| l.starts_with("drop_save")).count(),
            2
        );
        assert!(!calls.contains("inspect"), "{calls}");
        let c = t.fx.state.hub.client(&t.part).await.unwrap();
        let gone = c.call::<Value>("drop_save", json!({"save": "s_d1"})).await;
        assert!(matches!(
            gone,
            Err(parts::CallError::Part { code: -32002, .. })
        ));
    })
}

#[test]
fn run_names_are_optional_validated_metadata() {
    run(async {
        let fx = setup("names").await;
        let id = fx
            .state
            .store
            .create_run("original-task", &team())
            .await
            .unwrap();
        let url = format!("{}/api/runs/{id}/name", fx.base);
        let before = fx.state.store.runs().await.unwrap();
        assert_eq!(before[0].name, None);
        let response = fx
            .http
            .post(&url)
            .json(&json!({"name": " Demo: retail "}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let after = fx.state.store.runs().await.unwrap();
        assert_eq!(after[0].name.as_deref(), Some("Demo: retail"));
        assert_eq!(after[0].task_id, "original-task");
        invalid_run_names(&fx, &url).await;
    });
}

async fn invalid_run_names(fx: &Fx, url: &str) {
    for name in ["".to_string(), "x".repeat(301)] {
        let response = fx
            .http
            .post(url)
            .json(&json!({"name": name}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
    }
    let response = fx
        .http
        .post(format!("{}/api/runs/missing/name", fx.base))
        .json(&json!({"name": "Demo: absent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
