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
    app::{AppState, Config},
    model::*,
    providers::ProviderConfig,
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

struct Fx {
    state: Arc<AppState>,
    _hub: TestHub,
    _clean: Cleanup,
    base: String,
    http: reqwest::Client,
}

async fn setup(tag: &str, upstream: &FakeUpstream) -> Fx {
    setup_with(tag, upstream, |_| {}).await
}

async fn setup_with(tag: &str, upstream: &FakeUpstream, tweak: impl FnOnce(&mut Config)) -> Fx {
    setup_on(tag, TestHub::new(&[]), upstream, tweak).await
}

async fn setup_on(
    tag: &str,
    hub: TestHub,
    upstream: &FakeUpstream,
    tweak: impl FnOnce(&mut Config),
) -> Fx {
    let (store, clean) = app_support::meta_store("api", tag).await;
    let (state, base) =
        app_support::open_app(store, hub.hub.clone(), &upstream.base_url, tweak).await;
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

    async fn wait(&self, branch: &str) -> Value {
        for _ in 0..600 {
            let (_, b) = self.get(&format!("/api/branches/{branch}")).await;
            if b["status"] != "running" {
                return b;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("branch still running");
    }
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

const ORDER: &str = "#W7181492";
const USER: &str = "isabella_johansson_2152";

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
            json!({"order_id": ORDER}),
        )]),
        tool_msg(vec![(
            "t3",
            "handoff",
            json!({"note": "Isabella wants to exchange an item in #W7181492."}),
        )]),
    ]
}

fn read_order(id: &str) -> Value {
    tool_msg(vec![(id, "get_order_details", json!({"order_id": ORDER}))])
}

fn gold_args() -> Value {
    gold_action_args("27", "exchange_delivered_order_items")
}

fn gold_write() -> Value {
    tool_msg(vec![("w1", "exchange_delivered_order_items", gold_args())])
}

fn wrong_args() -> Value {
    json!({"user_id": USER, "address1": "1 Wrong St", "address2": "", "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"})
}

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
    tokio::time::timeout(Duration::from_secs(60), read)
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

/// The branch's stream is ordered without gaps, ends with `check` and a passed status. Returns
/// the number of frames and of events.
async fn assert_stream_passed(fx: &Fx, branch: &str) -> (usize, usize) {
    let fr = read_stream(fx, branch).await;
    let last = fr.last().unwrap();
    assert_eq!(last.0, "status");
    let status: Value = serde_json::from_str(&last.1).unwrap();
    assert_eq!(status["status"], "passed", "{status}");
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

/// The branch endpoint has all `n_events` and the run list shows the branch as passed.
async fn assert_branch_and_run_listed(fx: &Fx, branch: &str, run_id: &str, n_events: usize) {
    let (_, b) = fx.get(&format!("/api/branches/{branch}")).await;
    assert_eq!(b["events"].as_array().unwrap().len(), n_events);
    assert_eq!(b["status"], "passed");
    let (_, runs) = fx.get("/api/runs").await;
    let r = runs
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run_id)
        .unwrap();
    assert_eq!(r["branches"][0]["id"], branch);
    assert_eq!(r["branches"][0]["status"], "passed");
}

#[test]
fn post_run_then_stream_until_status() {
    run(async {
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("stream", &up).await;

        assert_health_and_tasks(&fx).await;
        assert_run_validation_errors(&fx).await;

        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_eq!(st, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let run_id = v["run_id"].as_str().unwrap().to_string();

        let (frames, seqs) = assert_stream_passed(&fx, &branch).await;
        // After the end, a new stream replays everything and ends with status immediately.
        let again = read_stream(&fx, &branch).await;
        assert_eq!(again.len(), frames);
        assert_branch_and_run_listed(&fx, &branch, &run_id, seqs).await;
        teardown(fx, &[&branch]).await;
    })
}

fn wrong_write() -> Value {
    tool_msg(vec![("w2", "modify_user_address", wrong_args())])
}

fn solo() -> Team {
    Team {
        agents: vec![agent("resolver")],
        ..team()
    }
}

/// Seq of the first tool call named `name` on `branch`.
async fn seq_of(fx: &Fx, branch: &str, name: &str) -> i64 {
    let evs = fx.state.store.events(branch).await.unwrap();
    evs.iter().find(|e| e.payload["name"] == name).unwrap().seq
}

/// A passed solo run (the gold exchange) and a failed one (a wrong address): `(a, b)`.
async fn passed_and_failed(fx: &Fx) -> (String, String) {
    let a = fx.run_to_end("27", &solo()).await;
    let b = fx.run_to_end("27", &solo()).await;
    (a, b)
}

/// The differing rows of a passed and a failed branch: the order and the user, each side's doc.
fn assert_passed_failed_rows(v: &Value) {
    let rows = v["differing_rows"].as_array().unwrap();
    let keys: Vec<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
    assert_eq!(
        keys,
        vec![format!("orders/{ORDER}"), format!("users/{USER}")]
    );
    assert_eq!(
        rows[0]["a"]["doc"]["status"], "exchange requested",
        "{}",
        rows[0]
    );
    assert_eq!(rows[0]["b"]["doc"]["status"], "delivered");
    assert_eq!(rows[1]["b"]["doc"]["address"]["address1"], "1 Wrong St");
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

/// Differing rows come from the parts' `diff` of both branches' final saves; the shape is
/// unchanged (`[{key, a, b}]`, keys without a part prefix in a one-part world).
#[test]
fn compare_differing_rows_unchanged_shape() {
    run(async {
        let script = vec![
            gold_write(),
            text_msg("Exchanged."),
            wrong_write(),
            text_msg("Moved."),
        ];
        let up = FakeUpstream::start(script).await;
        let fx = setup("compare", &up).await;
        let (a, b) = passed_and_failed(&fx).await;
        let (st, v) = fx.get(&format!("/api/compare?a={a}&b={b}")).await;
        assert_eq!(st, 200, "{v}");
        let statuses = (v["a"]["status"].as_str(), v["b"]["status"].as_str());
        assert_eq!(statuses, (Some("passed"), Some("failed")));
        assert!(
            v["a"]["tokens"].as_i64().unwrap() > 0 && v["a"]["seconds"].as_f64().unwrap() >= 0.0
        );
        let first_write = seq_of(&fx, &a, "exchange_delivered_order_items").await;
        assert_eq!(v["first_divergent_seq"], first_write);
        assert_passed_failed_rows(&v);
        compare_edge_cases(&fx, &a).await;
        teardown(fx, &[&a, &b]).await;
    })
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

/// The order and the user just before the gold write, after it, and after the wrong write.
async fn assert_rows_at_steps(fx: &Fx, b: &str, [s_gold, s_wrong]: [i64; 2]) {
    let keys = format!("orders/%23W7181492,users/{USER},users/nobody");
    let at = async |seq: i64, extra: &str| fx.get(&rows_path(b, seq, &keys, extra)).await;
    let doc = |v: &Value, key: &str, field: &str| v["rows"][key]["doc"][field].clone();
    let (order, user) = (format!("orders/{ORDER}"), format!("users/{USER}"));
    let (st, v) = at(s_gold - 1, "").await;
    assert_eq!((st, &v["seq"]), (200, &json!(s_gold - 1)), "{v}");
    assert_eq!(doc(&v, &order, "status"), "delivered", "{v}");
    assert!(v["rows"]["users/nobody"].is_null());
    let addr_before = doc(&v, &user, "address")["address1"].clone();
    assert!(addr_before.is_string(), "{v}");
    let (_, v) = at(s_gold, "").await;
    assert_eq!(doc(&v, &order, "status"), "exchange requested");
    assert_eq!(doc(&v, &user, "address")["address1"], addr_before);
    let (st, v) = at(s_wrong, "&part=retail").await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(doc(&v, &user, "address")["address1"], "1 Wrong St");
    assert_eq!(doc(&v, &order, "status"), "exchange requested");
}

/// Rows are read with `inspect` from the save holding the part's state after the step.
#[test]
fn rows_via_inspect_before_after() {
    run(async {
        let script = vec![gold_write(), wrong_write(), text_msg("Done.")];
        let up = FakeUpstream::start(script).await;
        let fx = setup("rows", &up).await;
        let b = fx.run_to_end("27", &solo()).await;
        let s_gold = seq_of(&fx, &b, "exchange_delivered_order_items").await;
        let s_wrong = seq_of(&fx, &b, "modify_user_address").await;
        assert_rows_at_steps(&fx, &b, [s_gold, s_wrong]).await;
        bad_rows_requests(&fx, &b, s_gold).await;
        teardown(fx, &[&b]).await;
    })
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

#[test]
fn fork_over_http_maps_errors_and_starts_a_branch() {
    run(async {
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), text_msg("Exchanged.")]);
        script.extend([read_order("f1"), gold_write(), text_msg("Again.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("fork", &up).await;
        let b = fx.run_to_end("27", &team()).await;
        let evs = fx.state.store.events(&b).await.unwrap();
        let at = evs
            .iter()
            .find(|e| {
                e.kind == EventKind::ModelCall
                    && e.payload["response"]["choices"][0]["message"]["tool_calls"][0]["function"]
                        ["name"]
                        == "exchange_delivered_order_items"
            })
            .unwrap()
            .seq;
        let url = format!("/api/branches/{b}/fork");
        assert_fork_errors(&fx, &url, at).await;
        let (st, v) = fx.post(&url, json!({"at_seq": at})).await;
        assert_eq!(st, 200, "{v}");
        let child = v["branch_id"].as_str().unwrap().to_string();
        assert_eq!(v["start_seq"], at);
        let done = fx.wait(&child).await;
        assert_eq!(done["status"], "passed", "{done}");
        assert_eq!(done["parent_id"], b.as_str());
        teardown(fx, &[&b, &child]).await;
    })
}

#[test]
fn open_mode_is_rejected_without_a_key() {
    run(async {
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup_with("nokey", &up, |c| {
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
        // A branch that stays running: a run row + running branch with no driver activity.
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup("shutdown", &up).await;
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
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup("nodb", &up).await;
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
        let up = FakeUpstream::start(vec![text_msg("x")]).await;
        let fx = setup("host", &up).await;
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

/// An app whose hub also has the toy integration `toy-<tag>`, for branches recorded by hand.
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
    let up = FakeUpstream::start(vec![]).await;
    let root = packages::toy_world(tag, part_args, &[]);
    let hub = TestHub::new(std::slice::from_ref(&root.0));
    let fx = setup_on(tag, hub, &up, |_| {}).await;
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
