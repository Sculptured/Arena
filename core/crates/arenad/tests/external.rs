//! A run an external system drives over HTTP: it starts the run, registers agents (one joins
//! mid-run), calls tools through each agent's MCP endpoint and models through its gateway, and
//! finishes the run; a fork hands it each agent's history. No real model: a fake upstream.
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
};

use app_support::send;
use arenad::{app::AppState, model::*};
use fake_upstream::FakeUpstream;
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

async fn setup(tag: &str, up: &FakeUpstream) -> Fx {
    let hub = TestHub::new(&[]);
    let (store, clean) = app_support::meta_store("external", tag).await;
    let (state, base) = app_support::open_app(store, hub.hub.clone(), &up.base_url, |_| {}).await;
    Fx {
        state,
        _hub: hub,
        _clean: clean,
        base,
        http: reqwest::Client::new(),
    }
}

impl Fx {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        send(self.http.post(format!("{}{path}", self.base)).json(&body)).await
    }

    /// Calls `tool` through an MCP endpoint, as an agent's MCP client would.
    async fn mcp(&self, url: &str, tool: &str, args: Value) -> Value {
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                         "params": {"name": tool, "arguments": args}});
        let (status, body) = send(self.http.post(url).json(&req)).await;
        assert_eq!(status, 200, "{body}");
        body["result"].clone()
    }

    /// `POST /api/branches/{branch}/{what}`.
    async fn on(&self, branch: &str, what: &str, body: Value) -> (u16, Value) {
        self.post(&format!("/api/branches/{branch}/{what}"), body)
            .await
    }

    async fn finish(&self, branch: &str) -> (u16, Value) {
        self.on(branch, "finish", json!({})).await
    }

    async fn events(&self, branch: &str) -> Vec<Event> {
        self.state.store.events(branch).await.unwrap()
    }
}

fn team(agents: Value) -> Value {
    json!({"id": "own", "name": "Own design", "agents": agents})
}

/// Starts an external run of task 27 with no agents yet: its branch id.
async fn start(fx: &Fx) -> String {
    let body = json!({"task_id": "27", "team": team(json!([])), "runner": "external"});
    let (status, v) = fx.post("/api/runs", body).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["agents"], json!([]));
    v["branch_id"].as_str().unwrap().to_string()
}

async fn register(fx: &Fx, branch: &str, join: Value) -> Value {
    let (status, v) = fx.on(branch, "agents", join).await;
    assert_eq!(status, 200, "{v}");
    v
}

/// The planner writes the gold exchange through its MCP endpoint and asks its model through its
/// gateway.
async fn planner_works(fx: &Fx, branch: &str) -> String {
    let planner = register(fx, branch, json!({"id": "planner", "name": "Planner"})).await;
    let tools = planner["tools"].to_string();
    assert!(tools.contains("exchange_delivered_order_items") && tools.contains("\"inbox\""));
    let mcp = planner["mcp_url"].as_str().unwrap().to_string();
    let args = gold_action_args("27", "exchange_delivered_order_items");
    let out = fx.mcp(&mcp, "exchange_delivered_order_items", args).await;
    assert_eq!(out["isError"], false);
    let chat = format!(
        "{}/chat/completions",
        planner["gateway_url"].as_str().unwrap()
    );
    let body = json!({"model": "fake/model", "messages": [{"role": "user", "content": "done?"}]});
    let (status, answer) = send(fx.http.post(chat).json(&body)).await;
    assert_eq!(status, 200, "{answer}");
    mcp
}

/// The planner works, the reviewer joins and is messaged, and the planner reports it is done.
async fn drive(fx: &Fx, branch: &str) -> Value {
    let planner_mcp = planner_works(fx, branch).await;
    let reviewer = register(fx, branch, json!({"id": "reviewer", "read_only": true})).await;
    let note = json!({"to": "reviewer", "text": "please check #W7181492"});
    assert_eq!(
        fx.mcp(&planner_mcp, "message", note).await["isError"],
        false
    );
    let end = json!({"prompt_tokens": 10, "completion_tokens": 5});
    assert_eq!(fx.on(branch, "agents/planner/end", end).await.0, 200);
    reviewer
}

fn kinds(evs: &[Event]) -> Vec<EventKind> {
    evs.iter().map(|e| e.kind).collect()
}

#[test]
fn an_external_system_drives_a_run_and_arena_records_it() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"role": "assistant", "content": "yes"})]).await;
        let fx = setup("drive", &up).await;
        let branch = start(&fx).await;
        let reviewer = drive(&fx, &branch).await;
        // The reviewer is read-only: it is offered no write tool.
        assert!(!reviewer["tools"]
            .to_string()
            .contains("exchange_delivered_order_items"));
        let (status, v) = fx.finish(&branch).await;
        assert_eq!((status, &v["status"]), (200, &json!("passed")), "{v}");
        let evs = fx.events(&branch).await;
        use EventKind::*;
        let want = [
            RunStart, AgentStart, ToolCall, ModelCall, AgentStart, Message, AgentEnd, Check,
        ];
        assert_eq!(kinds(&evs), want);
        assert_eq!(evs[0].payload["runner"], "external");
        let b = fx.state.store.branch(&branch).await.unwrap();
        let ids: Vec<_> = b.team.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["planner", "reviewer"]);
        assert!(b.team.agents.iter().all(|a| a.kind == AgentKind::External));
        assert_eq!(
            fx.finish(&branch).await.0,
            404,
            "a finished branch is no longer live"
        );
    })
}

fn reported_call(cached: i64) -> Value {
    json!({"message_id": format!("msg{cached}"), "model": "m", "input_tokens": 3,
           "cache_creation_input_tokens": 10, "cache_read_input_tokens": cached,
           "output_tokens": 4, "context_window": 200000})
}

/// The branch's metrics, from `GET /api/branches/{id}`.
async fn metrics_of(fx: &Fx, branch: &str) -> Value {
    let url = format!("{}/api/branches/{branch}", fx.base);
    let (status, b) = send(fx.http.get(url)).await;
    assert_eq!(status, 200, "{b}");
    b["metrics"].clone()
}

/// A CLI agent's client reports each model call (and a compaction); arena records them and its
/// metrics give the agent's calls and peak context.
#[test]
fn a_clients_reported_model_calls_are_recorded_with_their_context() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"role": "assistant", "content": "ok"})]).await;
        let fx = setup("usage", &up).await;
        let branch = start(&fx).await;
        register(&fx, &branch, json!({"id": "cli", "model": "m"})).await;
        let compaction = json!({"compaction": {"pre_tokens": 5017}});
        for body in [reported_call(100), reported_call(5000), compaction] {
            let (status, v) = fx.on(&branch, "agents/cli/usage", body).await;
            assert_eq!(status, 200, "{v}");
        }
        let unknown = fx.on(&branch, "agents/nobody/usage", reported_call(1));
        assert_eq!(unknown.await.0, 404);
        let evs = fx.events(&branch).await;
        let calls: Vec<&Event> = evs
            .iter()
            .filter(|e| e.kind == EventKind::ModelCall)
            .collect();
        assert_eq!(calls[1].payload["usage"]["prompt_tokens"], 5013);
        assert_eq!(calls[1].payload["reported_by"], "client");
        let a = metrics_of(&fx, &branch).await["agents"][0].clone();
        assert_eq!(
            (&a["model_calls"], &a["peak_prompt_tokens"]),
            (&json!(2), &json!(5013))
        );
        assert_eq!((&a["compactions"], &a["tokens"]), (&json!(1), &json!(5134)));
        assert_eq!(fx.finish(&branch).await.0, 200);
    })
}

#[test]
fn a_fork_of_an_external_run_hands_back_each_agents_history() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"role": "assistant", "content": "yes"})]).await;
        let fx = setup("fork", &up).await;
        let branch = start(&fx).await;
        drive(&fx, &branch).await;
        assert_eq!(fx.finish(&branch).await.0, 200);
        // Fork at the reviewer's start: after the planner's write and model call.
        let at = fx.events(&branch).await[4].seq;
        let (status, f) = fx.on(&branch, "fork", json!({"at_seq": at})).await;
        assert_eq!((status, &f["start_seq"]), (200, &json!(at)), "{f}");
        let agents = f["agents"].as_array().unwrap();
        let of = |id: &str| agents.iter().find(|a| a["agent"] == id).unwrap().clone();
        let history = of("planner")["history"].as_array().unwrap().clone();
        let kinds: Vec<_> = history
            .iter()
            .map(|e| e["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, ["agent_start", "tool_call", "model_call"]);
        let child = f["branch_id"].as_str().unwrap();
        assert!(fx.state.live.read().unwrap().contains_key(child));
        // The system resumes the reviewer on the new branch; the world kept the planner's write.
        assert!(of("reviewer")["mcp_url"].as_str().unwrap().contains(child));
        let (status, v) = fx.finish(child).await;
        assert_eq!((status, &v["status"]), (200, &json!("passed")), "{v}");
    })
}

/// A team of one external agent, `solo`.
fn single() -> Value {
    let solo = json!({"id": "solo", "name": "Solo", "kind": "external", "model": null,
                      "prompt": "You do it all."});
    json!({"id": "single", "name": "Single agent", "agents": [solo]})
}

/// A fork of an external run may give another team: the new branch runs it (its agents have no
/// history of their own), and asking for both a team and agent changes is refused.
#[test]
fn a_fork_of_an_external_run_can_change_the_team() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"role": "assistant", "content": "yes"})]).await;
        let fx = setup("swap", &up).await;
        let branch = start(&fx).await;
        drive(&fx, &branch).await;
        assert_eq!(fx.finish(&branch).await.0, 200);
        let at = fx.events(&branch).await[4].seq;
        let swap = json!({"at_seq": at, "team": single()});
        let both = json!({"changes": {"planner": {"prompt": "x"}}, "at_seq": at, "team": single()});
        let (status, v) = fx.on(&branch, "fork", both).await;
        assert_eq!(status, 400, "{v}");
        let (status, f) = fx.on(&branch, "fork", swap).await;
        assert_eq!(status, 200, "{f}");
        let agents = f["agents"].as_array().unwrap();
        assert_eq!((agents.len(), &agents[0]["agent"]), (1, &json!("solo")));
        let child = f["branch_id"].as_str().unwrap();
        let stored = fx.state.store.branch(child).await.unwrap();
        assert_eq!(stored.team.id, "single");
        assert!(stored.fork_note.unwrap().contains("Single agent"));
        let out = fx
            .mcp(
                agents[0]["mcp_url"].as_str().unwrap(),
                "get_order_details",
                json!({"order_id": "#W7181492"}),
            )
            .await;
        assert_eq!(out["isError"], false, "{out}");
        assert_eq!(fx.finish(child).await.0, 200);
    })
}

#[test]
fn only_external_runs_take_registrations() {
    run(async {
        let up = FakeUpstream::start(vec![json!({"role": "assistant", "content": "ok"})]).await;
        let fx = setup("refuse", &up).await;
        assert_eq!(fx.on("nope", "agents", json!({"id": "a"})).await.0, 404);
        let branch = start(&fx).await;
        for bad in ["link", "Bad Id"] {
            let (status, v) = fx.on(&branch, "agents", json!({"id": bad})).await;
            assert_eq!(status, 400, "{v}");
        }
        // An external agent can't be run by arenad's own runners.
        let agent =
            json!({"id": "x", "name": "X", "kind": "external", "model": null, "prompt": ""});
        let body = json!({"task_id": "27", "team": team(json!([agent]))});
        let (status, v) = fx.post("/api/runs", body).await;
        assert_eq!(status, 400, "{v}");
        assert_eq!(fx.finish(&branch).await.0, 200);
    })
}
