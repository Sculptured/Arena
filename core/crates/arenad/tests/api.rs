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
    app::{self, AppState, Config},
    model::*,
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
    let hub = TestHub::new(&[]);
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

#[test]
fn post_run_then_stream_until_status() {
    run(async {
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), text_msg("Exchanged.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("stream", &up).await;

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

        // Validation errors.
        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "nope", "team": team()}))
            .await;
        assert_eq!((st, v["error"].is_string()), (404, true));
        let dup = Team {
            agents: vec![agent("a"), agent("a")],
            ..team()
        };
        assert_eq!(
            fx.post("/api/runs", json!({"task_id": "27", "team": dup}))
                .await
                .0,
            400
        );
        let bad = Team {
            agents: vec![agent("Bad Id")],
            ..team()
        };
        assert_eq!(
            fx.post("/api/runs", json!({"task_id": "27", "team": bad}))
                .await
                .0,
            400
        );
        let none = Team {
            agents: vec![],
            ..team()
        };
        assert_eq!(
            fx.post("/api/runs", json!({"task_id": "27", "team": none}))
                .await
                .0,
            400
        );
        assert_eq!(fx.post("/api/runs", json!({"task_id": "27"})).await.0, 400);
        assert_eq!(fx.get("/api/branches/nope").await.0, 404);

        let (st, v) = fx
            .post("/api/runs", json!({"task_id": "27", "team": team()}))
            .await;
        assert_eq!(st, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let run_id = v["run_id"].as_str().unwrap().to_string();

        let fr = read_stream(&fx, &branch).await;
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

        // After the end, a new stream replays everything and ends with status immediately.
        let again = read_stream(&fx, &branch).await;
        assert_eq!(again.len(), fr.len());

        let (_, b) = fx.get(&format!("/api/branches/{branch}")).await;
        assert_eq!(b["events"].as_array().unwrap().len(), seqs.len());
        assert_eq!(b["status"], "passed");
        let (_, runs) = fx.get("/api/runs").await;
        let r = runs
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == run_id.as_str())
            .unwrap();
        assert_eq!(r["branches"][0]["id"], branch.as_str());
        assert_eq!(r["branches"][0]["status"], "passed");
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
        let keys = format!("orders/%23W7181492,users/{USER},users/nobody");
        let (order, user) = (format!("orders/{ORDER}"), format!("users/{USER}"));
        let at = async |seq: i64, extra: &str| fx.get(&rows_path(&b, seq, &keys, extra)).await;

        let (st, v) = at(s_gold - 1, "").await;
        assert_eq!((st, &v["seq"]), (200, &json!(s_gold - 1)), "{v}");
        assert_eq!(v["rows"][&order]["doc"]["status"], "delivered", "{v}");
        assert!(v["rows"]["users/nobody"].is_null());
        let addr_before = v["rows"][&user]["doc"]["address"]["address1"].clone();
        assert!(addr_before.is_string(), "{v}");

        let (_, v) = at(s_gold, "").await;
        assert_eq!(v["rows"][&order]["doc"]["status"], "exchange requested");
        assert_eq!(v["rows"][&user]["doc"]["address"]["address1"], addr_before);

        let (st, v) = at(s_wrong, "&part=retail").await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["rows"][&user]["doc"]["address"]["address1"], "1 Wrong St");
        assert_eq!(v["rows"][&order]["doc"]["status"], "exchange requested");
        bad_rows_requests(&fx, &b, s_gold).await;
        teardown(fx, &[&b]).await;
    })
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
        assert_eq!(fx.post(&url, json!({"at_seq": 9999})).await.0, 400);
        assert_eq!(
            fx.post(
                &url,
                json!({"at_seq": at, "changes": {"ghost": {"prompt": "x"}}})
            )
            .await
            .0,
            400
        );
        assert_eq!(fx.post(&url, json!({"nope": 1})).await.0, 400);
        assert_eq!(
            fx.post("/api/branches/nope/fork", json!({"at_seq": 1}))
                .await
                .0,
            404
        );
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
            c.openrouter_key = None;
            c.upstream_base = app::OPENROUTER_BASE.to_string();
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
        let (st, v) = fx
            .post(
                "/api/branches/whatever/fork",
                json!({"at_seq": 1, "changes": {"resolver": {"kind": "open"}}}),
            )
            .await;
        assert_eq!(
            (st, v["error"].as_str()),
            (400, Some("open mode needs OPENROUTER_API_KEY"))
        );
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
