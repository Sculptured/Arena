//! `http` agents (`arena-agent/1`, `src/agents/http.rs`) end to end, on the retail benchmark.
//!
//! The agent at the other end of the URL is a protocol client written here: it takes the run's
//! POST, calls the MCP endpoint it is given (`tools/list`, then one `tools/call`), and answers.
//! It decides nothing and no model is involved: these tests check the protocol (what arenad
//! sends, that the token guards the endpoint, that each call is recorded, how an answer, an error
//! or a timeout ends the run, that header values stay out of the record), not an agent.

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
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use arenad::{app::AppState, model::*};
use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use packages::TestHub;
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

const SECRET: &str = "s3cret-header-value";

/// What the protocol client saw: each request's `x-api-key` and body, and the status of an MCP
/// call made without the token.
#[derive(Default)]
struct Seen {
    keys: Vec<String>,
    bodies: Vec<Value>,
    without_token: Option<u16>,
}

type Shared = Arc<Mutex<Seen>>;

async fn mcp(url: &str, token: Option<&str>, body: Value) -> (u16, Value) {
    let mut req = reqwest::Client::new().post(url).json(&body);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    app_support::send(req).await
}

/// The run as the protocol client takes it: list the tools, call the first, answer done.
async fn work(seen: &Shared, run: &Value) -> Value {
    let url = run["mcp_url"].as_str().unwrap();
    let token = run["token"].as_str().unwrap();
    let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let (without, _) = mcp(url, None, list.clone()).await;
    seen.lock().unwrap().without_token = Some(without);
    let (_, tools) = mcp(url, Some(token), list).await;
    let first = tools["result"]["tools"][0]["name"].clone();
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                      "params": {"name": first, "arguments": {}}});
    mcp(url, Some(token), call).await;
    json!({"ok": true, "usage": {"input_tokens": 120, "output_tokens": 30}, "cost_usd": 0.002})
}

async fn answer(
    State(seen): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let key = headers
        .get("x-api-key")
        .map(|v| v.to_str().unwrap().to_string());
    {
        let mut s = seen.lock().unwrap();
        s.keys.extend(key);
        s.bodies.push(body.clone());
    }
    if body["type"] == "hello" {
        return Json(json!({"name": "Plant agent"}));
    }
    Json(work(&seen, &body).await)
}

/// The protocol client at `/ok`, an agent that never answers at `/slow` and one that fails at
/// `/error`; the server's base URL.
async fn agent_server(seen: Shared) -> String {
    let app = Router::new()
        .route("/ok", post(answer))
        .route(
            "/slow",
            post(|| async { std::future::pending::<String>().await }),
        )
        .route(
            "/error",
            post(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        )
        .with_state(seen);
    let (listener, base) = app_support::bind().await;
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

struct Fx {
    state: Arc<AppState>,
    base: String,
    agent: String,
    seen: Shared,
    _hub: TestHub,
    _clean: Cleanup,
}

async fn setup(tag: &str) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("http", tag).await;
    let (state, base) = app_support::idle_app(store, hub.hub.clone(), None, |c| {
        c.http_agent_timeout = Duration::from_secs(2);
    })
    .await;
    let seen = Shared::default();
    let agent = agent_server(seen.clone()).await;
    Fx {
        state,
        base,
        agent,
        seen,
        _hub: hub,
        _clean: clean,
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let req = reqwest::Client::new()
            .post(format!("{}{path}", self.base))
            .json(&body);
        app_support::send(req).await
    }

    fn member(&self, path: &str) -> Value {
        json!({"id": "remote", "name": "Remote", "kind": "http", "url": format!("{}{path}", self.agent),
               "headers": {"X-Api-Key": SECRET}})
    }

    /// Runs task 27 with `members` to its end; the branch and its events.
    async fn run_to_end(&self, members: Value, runner: &str) -> (Branch, Vec<Event>, Value) {
        let team = json!({"id": "t", "name": "T", "agents": members});
        let (st, v) = self
            .post(
                "/api/runs",
                json!({"task_id": "27", "team": team, "runner": runner}),
            )
            .await;
        assert_eq!(st, 200, "{v}");
        let id = v["branch_id"].as_str().unwrap();
        let b = live::ended(&self.state, id).await;
        let evs = self.state.store.events(id).await.unwrap();
        let shown = app_support::send(
            reqwest::Client::new().get(format!("{}/api/branches/{id}", self.base)),
        )
        .await
        .1;
        (b, evs, shown)
    }
}

fn agent_end(evs: &[Event]) -> &Value {
    &evs.iter()
        .find(|e| e.kind == EventKind::AgentEnd)
        .expect("an agent end")
        .payload
}

/// What arenad sent `b`'s agent: the protocol, the task, an MCP endpoint, a token, a deadline.
fn assert_run_body(sent: &Value, b: &Branch) {
    let fields = |k: [&str; 2]| (sent[k[0]].clone(), sent[k[1]].clone());
    assert_eq!(
        fields(["protocol", "type"]),
        (json!("arena-agent/1"), json!("run"))
    );
    assert_eq!(
        fields(["branch_id", "run_id"]),
        (json!(b.id), json!(b.run_id))
    );
    assert!(sent["task"].as_str().unwrap().contains('#'), "{sent}");
    let mcp_url = sent["mcp_url"].as_str().unwrap();
    assert!(mcp_url.ends_with(&format!("/mcp/{}/remote", b.id)));
    assert!(chrono::DateTime::parse_from_rfc3339(sent["deadline"].as_str().unwrap()).is_ok());
}

#[test]
fn an_http_agent_works_through_its_mcp_endpoint_and_every_call_is_recorded() {
    run(async {
        let fx = setup("work").await;
        let (b, evs, shown) = fx.run_to_end(json!([fx.member("/ok")]), "chain").await;
        let ended = matches!(b.status, BranchStatus::Passed | BranchStatus::Failed);
        assert!(ended, "{:?} {:?}", b.status, b.error);
        let (sent, keys, without_token) = {
            let seen = fx.seen.lock().unwrap();
            (
                seen.bodies[0].clone(),
                seen.keys.clone(),
                seen.without_token,
            )
        };
        assert_run_body(&sent, &b);
        assert_eq!(keys, [SECRET]);
        // The endpoint refused the call without the token; the call with it was recorded.
        assert_eq!(without_token, Some(401));
        assert_eq!(live::calls_by(&evs, "remote").len(), 1);
        let end = agent_end(&evs);
        let got = (&end["ok"], &end["tokens"], &end["cost_source"]);
        assert_eq!(got, (&json!(true), &json!(150), &json!("agent_reported")));
        // The header's value is in no record, and the token no longer opens the endpoint.
        let stored = serde_json::to_string(&(&evs, &shown, &b.team)).unwrap();
        assert!(!stored.contains(SECRET));
        let list = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
        let token = sent["token"].as_str();
        let (status, _) = mcp(sent["mcp_url"].as_str().unwrap(), token, list).await;
        assert_ne!(status, 200);
    });
}

#[test]
fn a_silent_failing_or_missing_agent_ends_the_run_with_its_reason() {
    run(async {
        let fx = setup("fail").await;
        let (b, evs, _) = fx.run_to_end(json!([fx.member("/slow")]), "chain").await;
        assert_eq!(b.status, BranchStatus::Failed);
        let host = fx.agent.trim_start_matches("http://");
        let why = format!("the agent at {host} did not answer within 2 s");
        assert_eq!(
            (agent_end(&evs)["error"].as_str(), b.error.as_deref()),
            (Some(why.as_str()), Some(why.as_str()))
        );

        let (b, _, _) = fx.run_to_end(json!([fx.member("/error")]), "chain").await;
        assert_eq!(b.status, BranchStatus::InfraError);
        assert_eq!(
            b.error.as_deref(),
            Some(format!("the agent at {host} answered HTTP 500: boom").as_str())
        );

        let down = app_support::bind().await.1;
        let gone =
            json!([{"id": "remote", "name": "R", "kind": "http", "url": format!("{down}/run")}]);
        let (b, _, _) = fx.run_to_end(gone, "chain").await;
        assert_eq!(b.status, BranchStatus::InfraError);
        assert!(b
            .error
            .unwrap()
            .starts_with("could not reach the agent at 127.0.0.1:"));
    });
}

#[test]
fn the_check_says_hello_and_gives_back_the_agent_without_its_secrets() {
    run(async {
        let fx = setup("check").await;
        let body = json!({"kind": "http", "url": format!("{}/ok", fx.agent), "headers": {"X-Api-Key": SECRET}});
        let (st, v) = fx.post("/api/agents/check", body).await;
        assert_eq!(st, 200, "{v}");
        assert_eq!(
            (&v["ok"], &v["name"], &v["agent"]["name"]),
            (&json!(true), &json!("Plant agent"), &json!("Plant agent"))
        );
        assert!(!v.to_string().contains(SECRET), "{v}");
        assert_eq!(
            fx.seen.lock().unwrap().bodies[0],
            json!({"protocol": "arena-agent/1", "type": "hello"})
        );
        // The agent it gave back runs: its header references still stand for the value.
        let (b, _, _) = fx.run_to_end(json!([v["agent"].clone()]), "chain").await;
        assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
        assert_eq!(fx.seen.lock().unwrap().keys, [SECRET, SECRET]);
    });
}

#[test]
fn a_check_that_fails_says_why() {
    run(async {
        let fx = setup("nocheck").await;
        let (_, v) = fx
            .post(
                "/api/agents/check",
                json!({"kind": "http", "url": format!("{}/error", fx.agent)}),
            )
            .await;
        assert_eq!(
            (&v["ok"], v["error"].as_str().unwrap().contains("HTTP 500")),
            (&json!(false), true)
        );
        let (_, v) = fx
            .post(
                "/api/agents/check",
                json!({"kind": "bedrock", "model_id": "m", "region": "Not a region"}),
            )
            .await;
        assert_eq!(v["ok"], false, "{v}");
        assert_eq!(
            fx.post("/api/agents/check", json!({"kind": "claude_code"}))
                .await
                .0,
            400
        );
    });
}

#[test]
fn teams_with_an_http_agent_are_refused_where_it_cannot_run() {
    run(async {
        let fx = setup("refuse").await;
        let start = |members: Value, runner: &str| {
            let team = json!({"id": "t", "name": "T", "agents": members});
            fx.post(
                "/api/runs",
                json!({"task_id": "27", "team": team, "runner": runner}),
            )
        };
        let (st, v) = start(json!([fx.member("/ok")]), "dispatch").await;
        assert_eq!(st, 400, "{v}");
        let bad_url = json!([{"id": "a", "name": "A", "kind": "http", "url": "ftp://x"}]);
        assert_eq!(start(bad_url, "chain").await.0, 400);
        let stale = json!([{"id": "a", "name": "A", "kind": "http", "url": fx.agent,
                            "headers": {"X-Api-Key": {"secret": "from-an-arenad-that-stopped"}}}]);
        let (st, v) = start(stale, "chain").await;
        assert_eq!(st, 400);
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains("connect the agent again"),
            "{v}"
        );
    });
}
