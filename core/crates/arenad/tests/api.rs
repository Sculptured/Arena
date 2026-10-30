#[path = "support/fake_upstream.rs"]
mod fake_upstream;

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use arenad::{
    agents::{open::OpenDriver, Driver},
    app::{self, AppState, Config},
    model::*,
    store::Store,
    tasks::Tasks,
};
use fake_upstream::FakeUpstream;
use serde_json::{json, Value};
use tokio::sync::OnceCell;
use world::{schema::load_base, snapshot::Worlds, testing};

static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

static WORLDS: OnceCell<Arc<Worlds>> = OnceCell::const_new();

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../data/retail")
}

async fn worlds() -> Arc<Worlds> {
    WORLDS
        .get_or_init(|| async {
            let w = Worlds::new(testing::admin());
            load_base(w.admin(), &data_dir()).await.unwrap();
            Arc::new(w)
        })
        .await
        .clone()
}

fn agent(id: &str) -> AgentSpec {
    AgentSpec { id: id.into(), name: id.into(), kind: AgentKind::Open, model: Some("fake/model".into()), prompt: format!("You are {id}.") }
}

fn team() -> Team {
    Team { id: "t".into(), name: "T".into(), agents: vec![agent("triage"), agent("resolver")] }
}

struct Fx {
    state: Arc<AppState>,
    meta: String,
    base: String,
    http: reqwest::Client,
}

async fn setup(tag: &str, upstream: &FakeUpstream) -> Fx {
    setup_with(tag, upstream, |_| {}).await
}

async fn setup_with(tag: &str, upstream: &FakeUpstream, tweak: impl FnOnce(&mut Config)) -> Fx {
    let worlds = worlds().await;
    let meta = format!("arena_test_api_{}_{}", tag, &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let store = Arc::new(Store::open_named(worlds.admin(), &meta).await.unwrap());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = Config::new(data_dir());
    config.upstream_base = upstream.base_url.clone();
    config.openrouter_key = Some("sk-secret".into());
    config.self_base = format!("http://127.0.0.1:{port}");
    tweak(&mut config);
    let state = Arc::new(AppState::new(store, worlds, config));
    let d: Arc<dyn Driver> = Arc::new(OpenDriver::new(state.clone()));
    assert!(state.install_drivers([(AgentKind::Open, d)].into_iter().collect()));
    let app = app::router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fx { state, meta, base: format!("http://127.0.0.1:{port}"), http: reqwest::Client::new() }
}

/// Drops the branches' databases, the snapshots their events name, and the metadata database.
async fn teardown(fx: Fx, branches: &[&str]) {
    let worlds = fx.state.worlds.clone();
    let mut snaps = vec![];
    for b in branches {
        for e in fx.state.store.events(b).await.unwrap() {
            if let Some(s) = e.payload.get("snapshot").and_then(Value::as_str) {
                snaps.push(s.to_string());
            }
        }
    }
    snaps.sort();
    snaps.dedup();
    for s in snaps {
        worlds.drop_db(&s).await.unwrap();
    }
    for b in branches {
        worlds.drop_db(&format!("branch_{b}")).await.unwrap();
    }
    drop(fx.state);
    worlds.admin().drop_db(&fx.meta).await.unwrap();
}

impl Fx {
    async fn get(&self, path: &str) -> (u16, Value) {
        let r = self.http.get(format!("{}{path}", self.base)).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let r = self.http.post(format!("{}{path}", self.base)).json(&body).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap())
    }

    async fn run_to_end(&self, task: &str, team: &Team) -> String {
        let (st, v) = self.post("/api/runs", json!({"task_id": task, "team": team})).await;
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
        tool_msg(vec![("t1", "find_user_id_by_name_zip", json!({"first_name": "Isabella", "last_name": "Johansson", "zip": "32286"}))]),
        tool_msg(vec![("t2", "get_order_details", json!({"order_id": ORDER}))]),
        tool_msg(vec![("t3", "handoff", json!({"note": "Isabella wants to exchange an item in #W7181492."}))]),
    ]
}

fn read_order(id: &str) -> Value {
    tool_msg(vec![(id, "get_order_details", json!({"order_id": ORDER}))])
}

fn gold_write() -> Value {
    let tasks = Tasks::load(&data_dir()).unwrap();
    let t = tasks.get("27").unwrap();
    let a = t["evaluation_criteria"]["actions"].as_array().unwrap().iter().find(|a| a["name"] == "exchange_delivered_order_items").unwrap();
    tool_msg(vec![("w1", "exchange_delivered_order_items", a["arguments"].clone())])
}

fn wrong_write() -> Value {
    tool_msg(vec![(
        "w2",
        "modify_user_address",
        json!({"user_id": USER, "address1": "1 Wrong St", "address2": "", "city": "Austin", "state": "TX", "country": "USA", "zip": "73301"}),
    )])
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
    let mut resp = fx.http.get(format!("{}/api/branches/{branch}/stream", fx.base)).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    assert!(resp.headers()["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let mut body = String::new();
    let read = async {
        while let Some(chunk) = resp.chunk().await.unwrap() {
            body.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
            if frames(&body).iter().any(|(n, _)| n == "status") {
                break;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), read).await.expect("no status frame");
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
        let (_, tasks) = fx.get("/api/tasks").await;
        assert!(tasks.as_array().unwrap().iter().any(|t| t["id"] == "27"));
        let (_, dt) = fx.get("/api/team/default").await;
        assert_eq!(dt["agents"].as_array().unwrap().len(), 2);

        // Validation errors.
        let (st, v) = fx.post("/api/runs", json!({"task_id": "nope", "team": team()})).await;
        assert_eq!((st, v["error"].is_string()), (404, true));
        let dup = Team { agents: vec![agent("a"), agent("a")], ..team() };
        assert_eq!(fx.post("/api/runs", json!({"task_id": "27", "team": dup})).await.0, 400);
        let bad = Team { agents: vec![agent("Bad Id")], ..team() };
        assert_eq!(fx.post("/api/runs", json!({"task_id": "27", "team": bad})).await.0, 400);
        let none = Team { agents: vec![], ..team() };
        assert_eq!(fx.post("/api/runs", json!({"task_id": "27", "team": none})).await.0, 400);
        assert_eq!(fx.post("/api/runs", json!({"task_id": "27"})).await.0, 400);
        assert_eq!(fx.get("/api/branches/nope").await.0, 404);

        let (st, v) = fx.post("/api/runs", json!({"task_id": "27", "team": team()})).await;
        assert_eq!(st, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let run_id = v["run_id"].as_str().unwrap().to_string();

        let fr = read_stream(&fx, &branch).await;
        let last = fr.last().unwrap();
        assert_eq!(last.0, "status");
        let status: Value = serde_json::from_str(&last.1).unwrap();
        assert_eq!(status["status"], "passed", "{status}");
        let evs: Vec<Value> = fr.iter().filter(|(n, _)| n == "event").map(|(_, d)| serde_json::from_str(d).unwrap()).collect();
        let seqs: Vec<i64> = evs.iter().map(|e| e["seq"].as_i64().unwrap()).collect();
        assert_eq!(seqs, (1..=seqs.len() as i64).collect::<Vec<_>>(), "ordered, no gaps or duplicates");
        assert_eq!(evs.last().unwrap()["kind"], "check");

        // After the end, a new stream replays everything and ends with status immediately.
        let again = read_stream(&fx, &branch).await;
        assert_eq!(again.len(), fr.len());

        let (_, b) = fx.get(&format!("/api/branches/{branch}")).await;
        assert_eq!(b["events"].as_array().unwrap().len(), seqs.len());
        assert_eq!(b["status"], "passed");
        let (_, runs) = fx.get("/api/runs").await;
        let r = runs.as_array().unwrap().iter().find(|r| r["id"] == run_id.as_str()).unwrap();
        assert_eq!(r["branches"][0]["id"], branch.as_str());
        assert_eq!(r["branches"][0]["status"], "passed");
        teardown(fx, &[&branch]).await;
    })
}

#[test]
fn compare_reports_first_divergence_and_rows() {
    run(async {
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), text_msg("Exchanged.")]);
        script.extend(triage_script());
        script.extend([read_order("r2"), wrong_write(), text_msg("Done.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("compare", &up).await;
        let a = fx.run_to_end("27", &team()).await;
        let b = fx.run_to_end("27", &team()).await;

        let (st, v) = fx.get(&format!("/api/compare?a={a}&b={b}")).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["a"]["status"], "passed");
        assert_eq!(v["b"]["status"], "failed");
        assert!(v["a"]["tokens"].as_i64().unwrap() > 0);
        assert!(v["a"]["seconds"].as_f64().unwrap() >= 0.0);

        let aevs = fx.state.store.events(&a).await.unwrap();
        let first_write = aevs.iter().find(|e| e.payload["name"] == "exchange_delivered_order_items").unwrap().seq;
        assert_eq!(v["first_divergent_seq"], first_write);

        let rows = v["differing_rows"].as_array().unwrap();
        let keys: Vec<&str> = rows.iter().map(|r| r["key"].as_str().unwrap()).collect();
        assert_eq!(keys, vec![format!("orders/{ORDER}"), format!("users/{USER}")]);
        assert_eq!(rows[0]["a"]["status"], "exchange requested");
        assert_eq!(rows[0]["b"]["status"], "delivered");
        assert_eq!(rows[1]["b"]["address"]["address1"], "1 Wrong St");

        // Identical branches: no divergence, no differing rows.
        let (_, same) = fx.get(&format!("/api/compare?a={a}&b={a}")).await;
        assert!(same["first_divergent_seq"].is_null());
        assert!(same["differing_rows"].as_array().unwrap().is_empty());
        assert_eq!(fx.get(&format!("/api/compare?a={a}&b=nope")).await.0, 404);
        teardown(fx, &[&a, &b]).await;
    })
}

#[test]
fn rows_at_seq_shows_state_after_that_step() {
    run(async {
        let mut script = triage_script();
        script.extend([read_order("r1"), gold_write(), wrong_write(), text_msg("Done.")]);
        let up = FakeUpstream::start(script).await;
        let fx = setup("rows", &up).await;
        let b = fx.run_to_end("27", &team()).await;
        let evs = fx.state.store.events(&b).await.unwrap();
        let seq_of = |name: &str| evs.iter().find(|e| e.payload["name"] == name).unwrap().seq;
        let (s_gold, s_wrong) = (seq_of("exchange_delivered_order_items"), seq_of("modify_user_address"));
        let keys = format!("orders/%23W7181492,users/{USER},users/nobody");

        let (st, v) = fx.get(&format!("/api/branches/{b}/rows?seq={}&keys={keys}", s_gold - 1)).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(v["seq"], s_gold - 1);
        assert_eq!(v["rows"][format!("orders/{ORDER}")]["status"], "delivered");
        assert!(v["rows"]["users/nobody"].is_null());
        let addr_before = v["rows"][format!("users/{USER}")]["address"]["address1"].clone();

        let (_, v) = fx.get(&format!("/api/branches/{b}/rows?seq={s_gold}&keys={keys}")).await;
        assert_eq!(v["rows"][format!("orders/{ORDER}")]["status"], "exchange requested");
        assert_eq!(v["rows"][format!("users/{USER}")]["address"]["address1"], addr_before);

        let (_, v) = fx.get(&format!("/api/branches/{b}/rows?seq={s_wrong}&keys={keys}")).await;
        assert_eq!(v["rows"][format!("users/{USER}")]["address"]["address1"], "1 Wrong St");
        assert_eq!(v["rows"][format!("orders/{ORDER}")]["status"], "exchange requested");

        // Bad requests; none leaves a temp database behind.
        assert_eq!(fx.get(&format!("/api/branches/{b}/rows?seq={s_gold}&keys=sessions/x")).await.0, 400);
        assert_eq!(fx.get(&format!("/api/branches/{b}/rows?seq=9999&keys=users/x")).await.0, 400);
        assert_eq!(fx.get("/api/branches/nope/rows?seq=1&keys=users/x").await.0, 404);
        let n = fx
            .state
            .worlds
            .admin()
            .with_db("postgres", async |c| {
                let r = c.query_one("SELECT count(*) FROM pg_database WHERE datname LIKE $1", &[&format!("branch_rows_{b}%")]).await?;
                Ok(r.get::<_, i64>(0))
            })
            .await
            .unwrap();
        assert_eq!(n, 0, "temp branch databases are dropped");
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
            .find(|e| e.kind == EventKind::ModelCall && e.payload["response"]["choices"][0]["message"]["tool_calls"][0]["function"]["name"] == "exchange_delivered_order_items")
            .unwrap()
            .seq;
        let url = format!("/api/branches/{b}/fork");
        assert_eq!(fx.post(&url, json!({"at_seq": 9999})).await.0, 400);
        assert_eq!(fx.post(&url, json!({"at_seq": at, "changes": {"ghost": {"prompt": "x"}}})).await.0, 400);
        assert_eq!(fx.post(&url, json!({"nope": 1})).await.0, 400);
        assert_eq!(fx.post("/api/branches/nope/fork", json!({"at_seq": 1})).await.0, 404);
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
        let (st, v) = fx.post("/api/runs", json!({"task_id": "27", "team": team()})).await;
        assert_eq!((st, v["error"].as_str()), (400, Some("open mode needs OPENROUTER_API_KEY")));
        let (st, v) = fx.post("/api/branches/whatever/fork", json!({"at_seq": 1, "changes": {"resolver": {"kind": "open"}}})).await;
        assert_eq!((st, v["error"].as_str()), (400, Some("open mode needs OPENROUTER_API_KEY")));
        assert!(fx.state.store.runs().await.unwrap().is_empty(), "no run was created");
        drop(fx.state);
        worlds().await.admin().drop_db(&fx.meta).await.unwrap();
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
        let b = fx.state.store.create_branch(&run_id, None, &t, &json!([]), None).await.unwrap();
        let mut resp = fx.http.get(format!("{}/api/branches/{b}/stream", fx.base)).send().await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        fx.state.shutdown.send(true).unwrap();
        let ended = tokio::time::timeout(Duration::from_secs(5), async { while resp.chunk().await.unwrap().is_some() {} }).await;
        assert!(ended.is_ok(), "stream did not end on shutdown");
        drop(fx.state);
        worlds().await.admin().drop_db(&fx.meta).await.unwrap();
    })
}
