//! Pausing, resuming and stopping a live run over HTTP. The run is one an external system drives
//! (the test makes the agent's calls through its MCP endpoint and gateway, as its client would);
//! the gateway's provider is a closed port, so a model call that goes through fails to connect.
//! No model answers anything here: these tests check the holding, not an agent.
#[path = "support/app.rs"]
mod app_support;
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
use arenad::{app::AppState, model::*};
use packages::{gold_action_args, TestHub};
use pg::Cleanup;
use serde_json::{json, Value};

static RT: LazyLock<tokio::runtime::Runtime> =
    LazyLock::new(|| tokio::runtime::Runtime::new().unwrap());

fn run<F: Future>(f: F) -> F::Output {
    RT.block_on(f)
}

struct Fx {
    state: Arc<AppState>,
    _hub: TestHub,
    _clean: Cleanup,
    base: String,
    http: reqwest::Client,
}

async fn setup(tag: &str) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("pause", tag).await;
    let (state, base) = app_support::idle_app(store, hub.hub.clone(), None, |_| {}).await;
    Fx {
        state,
        _hub: hub,
        _clean: clean,
        base,
        http: reqwest::Client::new(),
    }
}

/// An agent's endpoints on a live branch.
struct Agent {
    mcp: String,
    chat: String,
}

impl Fx {
    /// `POST /api/branches/{branch}/{what}` (pause or resume) answers `{paused, changed}`.
    async fn flow(&self, branch: &str, what: &str, paused: bool, changed: bool) {
        let (status, v) = self.on(branch, what).await;
        let want = json!({"paused": paused, "changed": changed});
        assert_eq!((status, &v), (200, &want), "{what}");
        assert_eq!(self.branch(branch).await["paused"], paused);
    }

    async fn on(&self, branch: &str, what: &str) -> (u16, Value) {
        let url = format!("{}/api/branches/{branch}/{what}", self.base);
        send(self.http.post(url).json(&json!({}))).await
    }

    async fn branch(&self, branch: &str) -> Value {
        let url = format!("{}/api/branches/{branch}", self.base);
        let (status, v) = send(self.http.get(url)).await;
        assert_eq!(status, 200, "{v}");
        v
    }

    /// Starts an external run of task 27 and registers its one agent.
    async fn start(&self) -> (String, Agent) {
        let team = json!({"id": "own", "name": "Own design", "agents": []});
        let body = json!({"task_id": "27", "team": team, "runner": "external"});
        let url = format!("{}/api/runs", self.base);
        let (status, v) = send(self.http.post(url).json(&body)).await;
        assert_eq!(status, 200, "{v}");
        let branch = v["branch_id"].as_str().unwrap().to_string();
        let url = format!("{}/api/branches/{branch}/agents", self.base);
        let join = json!({"id": "planner", "name": "Planner"});
        let (status, a) = send(self.http.post(url).json(&join)).await;
        assert_eq!(status, 200, "{a}");
        let agent = Agent {
            mcp: a["mcp_url"].as_str().unwrap().to_string(),
            chat: format!("{}/chat/completions", a["gateway_url"].as_str().unwrap()),
        };
        (branch, agent)
    }

    async fn kinds(&self, branch: &str) -> Vec<EventKind> {
        let evs = self.state.store.events(branch).await.unwrap();
        evs.iter().map(|e| e.kind).collect()
    }

    fn clock_ms(&self, branch: &str) -> i64 {
        self.state.live.read().unwrap()[branch].clock.now_ms()
    }
}

/// The gold write of task 27 through the agent's MCP endpoint, on its own task: the call's
/// `tools/call` result.
fn tool_call(fx: &Fx, agent: &Agent) -> tokio::task::JoinHandle<Value> {
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                     "params": {"name": "exchange_delivered_order_items",
                                "arguments": gold_action_args("27", "exchange_delivered_order_items")}});
    let post = fx.http.post(&agent.mcp).json(&req);
    tokio::spawn(async move { send(post).await.1["result"].clone() })
}

/// A model call through the agent's gateway, on its own task: its HTTP status.
fn model_call(fx: &Fx, agent: &Agent) -> tokio::task::JoinHandle<u16> {
    let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    let post = fx.http.post(&agent.chat).json(&body);
    tokio::spawn(async move { post.send().await.unwrap().status().as_u16() })
}

const HELD: Duration = Duration::from_millis(400);

/// `Pause` and `Resume` are recorded around the held call, with the pause's length, which is
/// no part of the run's estimate in production.
async fn pause_is_recorded(fx: &Fx, branch: &str) {
    let evs = fx.state.store.events(branch).await.unwrap();
    let at = |k| evs.iter().position(|e| e.kind == k).unwrap();
    use EventKind::*;
    assert!(at(Pause) < at(Resume) && at(Resume) < at(ToolCall));
    let paused_ms = evs[at(Resume)].payload["paused_ms"].as_i64().unwrap();
    assert!(paused_ms >= HELD.as_millis() as i64, "{paused_ms}");
    let pause = evs[at(Resume)].payload["overhead_ms"]["pause"]
        .as_f64()
        .unwrap();
    assert!(pause >= HELD.as_millis() as f64, "{pause}");
}

#[test]
fn a_paused_run_holds_its_calls_and_clock_until_it_resumes() {
    run(async {
        let fx = setup("hold").await;
        let (branch, agent) = fx.start().await;
        fx.flow(&branch, "pause", true, true).await;
        fx.flow(&branch, "pause", true, false).await;
        let clock = fx.clock_ms(&branch);
        let tool = tool_call(&fx, &agent);
        let model = model_call(&fx, &agent);
        tokio::time::sleep(HELD).await;
        assert!(!tool.is_finished(), "a tool call waits while paused");
        assert!(!model.is_finished(), "a model call waits while paused");
        assert_eq!(fx.clock_ms(&branch), clock, "the world clock stands still");
        assert!(!fx.kinds(&branch).await.contains(&EventKind::ToolCall));

        fx.flow(&branch, "resume", false, true).await;
        assert_eq!(tool.await.unwrap()["isError"], false, "the held call ran");
        // Let through, the model call reaches its provider (a closed port): the gateway's error.
        assert_eq!(model.await.unwrap(), 502);
        fx.flow(&branch, "resume", false, false).await;

        pause_is_recorded(&fx, &branch).await;
        let url = format!("{}/api/branches/{branch}/finish", fx.base);
        let (status, v) = send(fx.http.post(url)).await;
        assert_eq!((status, &v["status"]), (200, &json!("passed")), "{v}");
        assert_eq!(
            fx.on(&branch, "pause").await.0,
            409,
            "an ended run can't pause"
        );
    })
}

#[test]
fn stopping_refuses_the_held_calls_and_ends_the_run_unscored() {
    run(async {
        let fx = setup("stop").await;
        let (branch, agent) = fx.start().await;
        assert_eq!(fx.on(&branch, "pause").await.0, 200);
        let tool = tool_call(&fx, &agent);
        let model = model_call(&fx, &agent);
        tokio::time::sleep(Duration::from_millis(100)).await;

        let (status, v) = fx.on(&branch, "stop").await;
        assert_eq!((status, &v), (200, &json!({"stopping": true})));
        let out = tool.await.unwrap();
        assert_eq!(out["isError"], true);
        assert!(out.to_string().contains("the run was stopped"), "{out}");
        assert_eq!(model.await.unwrap(), 409);

        let b = fx.state.store.branch(&branch).await.unwrap();
        assert_eq!((b.status, b.error), (BranchStatus::Stopped, None));
        assert!(
            !fx.kinds(&branch).await.contains(&EventKind::Check),
            "not scored"
        );
        assert!(!fx.state.live.read().unwrap().contains_key(&branch));
        assert_eq!(fx.branch(&branch).await["paused"], false);
        for what in ["pause", "resume", "stop"] {
            assert_eq!(fx.on(&branch, what).await.0, 409, "{what}");
        }
        assert_eq!(fx.on("no-such-branch", "pause").await.0, 404);
    })
}
