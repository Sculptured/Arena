//! A run an external system drives over HTTP: it starts the run, registers its agents, runs each
//! one itself against the agent's MCP endpoint, reports the model calls its client saw, ends the
//! agents and finishes the run; a fork hands it each agent's history. Without a model: what is
//! refused. `live_`: the agent is Claude Code (signed in; see `support/live.rs`), run by this
//! test the way a design in its own code runs it.
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
};

use app_support::send;
use arenad::{app::AppState, model::*};
use live::Live;
use packages::TestHub;
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
    let (store, clean) = app_support::meta_store("external", tag).await;
    let (state, base) = app_support::idle_app(store, hub.hub.clone(), None, |_| {}).await;
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

    /// The branch's metrics, from `GET /api/branches/{id}`.
    async fn metrics_of(&self, branch: &str) -> Value {
        let url = format!("{}/api/branches/{branch}", self.base);
        let (status, b) = send(self.http.get(url)).await;
        assert_eq!(status, 200, "{b}");
        b["metrics"].clone()
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

fn kinds(evs: &[Event]) -> Vec<EventKind> {
    evs.iter().map(|e| e.kind).collect()
}

#[test]
fn only_external_runs_take_registrations() {
    run(async {
        let fx = setup("refuse").await;
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
        let unknown = fx.on(&branch, "agents/nobody/usage", json!({"input_tokens": 1}));
        assert_eq!(unknown.await.0, 404);
        assert_eq!(fx.finish(&branch).await.0, 200);
        assert_eq!(fx.finish(&branch).await.0, 404, "a finished branch is no longer live");
    })
}

// ---------------------------------------------------------------- Claude Code as the agent

/// The ticket of task 27.
async fn ticket(fx: &Fx) -> String {
    let bench = fx.state.hub.bench_describe("retail").await.unwrap();
    let t = bench.tasks.iter().find(|t| t.id == "27").unwrap();
    t.ticket.clone()
}

/// Registers `id` and runs Claude Code as it on the ticket through its MCP endpoint, reporting
/// each model call the client saw and the agent's end, as a design does. The registration and
/// what was reported.
async fn work(fx: &Fx, live: &Live, branch: &str, join: Value, task: &str) -> (Value, live::ExternalRun) {
    let id = join["id"].as_str().unwrap().to_string();
    let reg = register(fx, branch, join).await;
    let mcp = reg["mcp_url"].as_str().unwrap();
    let prompt = format!("You are a retail support agent. Use your tools to handle this ticket, then stop.\n\n{task}");
    let ran = live::run_claude(live, "retail", mcp, &prompt).await;
    for r in &ran.reports {
        let (status, v) = fx.on(branch, &format!("agents/{id}/usage"), r.clone()).await;
        assert_eq!(status, 200, "{v}");
    }
    let ok = ran.result["is_error"] == false;
    let end = json!({"ok": ok, "prompt_tokens": ran.prompt_tokens,
                     "completion_tokens": ran.completion_tokens,
                     "cost_usd": ran.result["total_cost_usd"]});
    assert_eq!(fx.on(branch, &format!("agents/{id}/end"), end).await.0, 200);
    (reg, ran)
}

/// The agent's model calls as its client reported them: one per message, each with its usage,
/// and the metrics' count and peak context are theirs.
async fn assert_reported(fx: &Fx, branch: &str, ran: &live::ExternalRun) {
    let evs = fx.events(branch).await;
    let calls: Vec<&Event> = evs.iter().filter(|e| e.kind == EventKind::ModelCall).collect();
    assert_eq!(calls.len(), ran.reports.len());
    assert!(!calls.is_empty(), "Claude Code made at least one model call");
    for (c, r) in calls.iter().zip(&ran.reports) {
        assert_eq!(c.payload["reported_by"], "client");
        assert_eq!(c.payload["message_id"], r["message_id"]);
        let whole = ["input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
            .iter()
            .map(|k| r[k].as_i64().unwrap())
            .sum::<i64>();
        assert_eq!(c.payload["usage"]["prompt_tokens"], whole);
    }
    let peak = calls.iter().map(|c| c.payload["usage"]["prompt_tokens"].as_i64().unwrap()).max();
    let a = fx.metrics_of(branch).await["agents"][0].clone();
    assert_eq!(a["model_calls"], json!(calls.len()));
    assert_eq!(a["peak_prompt_tokens"], json!(peak.unwrap()));
}

/// An external system drives a run with Claude Code as its agent: Arena records the run's start,
/// the agent's start, each tool call it made through its endpoint (with its fields), each model
/// call its client reported, its end, and the check when the run is finished. A read-only agent
/// that joins later is offered no write tool.
#[test]
#[ignore = "live: runs Claude Code (support/live.rs)"]
fn live_an_external_system_drives_a_run_and_arena_records_it() {
    let Some(live) = Live::claude("live_an_external_system_drives_a_run_and_arena_records_it") else {
        return;
    };
    run(async {
        let fx = setup("drive").await;
        let branch = start(&fx).await;
        let task = ticket(&fx).await;
        let (reg, ran) = work(&fx, &live, &branch, json!({"id": "solo", "model": live.model()}), &task).await;
        let tools = reg["tools"].to_string();
        assert!(tools.contains("exchange_delivered_order_items") && tools.contains("\"inbox\""));
        let reviewer = register(&fx, &branch, json!({"id": "reviewer", "read_only": true})).await;
        assert!(!reviewer["tools"].to_string().contains("exchange_delivered_order_items"));
        let offered = live::mcp_tools(reviewer["mcp_url"].as_str().unwrap()).await;
        assert!(!offered.iter().any(|t| t == "exchange_delivered_order_items"), "{offered:?}");
        assert_reported(&fx, &branch, &ran).await;
        let (status, v) = fx.finish(&branch).await;
        assert_eq!(status, 200, "{v}");
        let b = fx.state.store.branch(&branch).await.unwrap();
        assert!(matches!(b.status, BranchStatus::Passed | BranchStatus::Failed), "{:?}", b.status);
        assert_eq!(v["status"], json!(b.status));
        let evs = fx.events(&branch).await;
        live::assert_recorded(&b, &evs, &live);
        assert_eq!(evs[0].payload["runner"], "external");
        let k = kinds(&evs);
        assert_eq!(k[..2], [EventKind::RunStart, EventKind::AgentStart]);
        assert_eq!(k.last(), Some(&EventKind::Check));
        let ids: Vec<_> = b.team.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["solo", "reviewer"]);
        assert!(b.team.agents.iter().all(|a| a.kind == AgentKind::External));
        assert_eq!(fx.finish(&branch).await.0, 404, "a finished branch is no longer live");
    })
}

/// A fork of an external run hands the system each agent's history up to the fork point and its
/// endpoints on the new branch, whose world kept what was done before it; a fork may give
/// another team instead (not with agent changes too), whose agents have no history.
#[test]
#[ignore = "live: runs Claude Code (support/live.rs)"]
fn live_a_fork_of_an_external_run_hands_back_each_agents_history() {
    let Some(live) = Live::claude("live_a_fork_of_an_external_run_hands_back_each_agents_history") else {
        return;
    };
    run(async {
        let fx = setup("fork").await;
        let branch = start(&fx).await;
        let task = ticket(&fx).await;
        work(&fx, &live, &branch, json!({"id": "solo", "model": live.model()}), &task).await;
        register(&fx, &branch, json!({"id": "reviewer", "read_only": true})).await;
        assert_eq!(fx.finish(&branch).await.0, 200);
        let evs = fx.events(&branch).await;
        // Fork at the reviewer's start: after everything the first agent did.
        let at = live::first_of(&evs, EventKind::AgentStart, "reviewer").unwrap();
        let (status, f) = fx.on(&branch, "fork", json!({"at_seq": at})).await;
        assert_eq!((status, &f["start_seq"]), (200, &json!(at)), "{f}");
        let agents = f["agents"].as_array().unwrap();
        let of = |id: &str| agents.iter().find(|a| a["agent"] == id).unwrap().clone();
        let history = of("solo")["history"].as_array().unwrap().clone();
        let want: Vec<Value> = evs
            .iter()
            .filter(|e| e.seq < at && e.agent.as_deref() == Some("solo"))
            .map(|e| json!(e.kind))
            .collect();
        let got: Vec<Value> = history.iter().map(|e| e["kind"].clone()).collect();
        assert_eq!(got, want);
        let child = f["branch_id"].as_str().unwrap().to_string();
        assert!(fx.state.live.read().unwrap().contains_key(&child));
        assert!(of("reviewer")["mcp_url"].as_str().unwrap().contains(&child));
        let (status, v) = fx.finish(&child).await;
        assert_eq!(status, 200, "{v}");
        let swap_team = json!({"id": "single", "name": "Single agent", "agents": [
            {"id": "solo2", "name": "Solo", "kind": "external", "model": null, "prompt": "You do it all."}]});
        let both = json!({"changes": {"solo": {"prompt": "x"}}, "at_seq": at, "team": swap_team});
        assert_eq!(fx.on(&branch, "fork", both).await.0, 400);
        let (status, f) = fx.on(&branch, "fork", json!({"at_seq": at, "team": swap_team})).await;
        assert_eq!(status, 200, "{f}");
        let agents = f["agents"].as_array().unwrap();
        assert_eq!((agents.len(), &agents[0]["agent"]), (1, &json!("solo2")));
        assert_eq!(agents[0]["history"], json!([]));
        let swapped = f["branch_id"].as_str().unwrap();
        let stored = fx.state.store.branch(swapped).await.unwrap();
        assert_eq!(stored.team.id, "single");
        assert!(stored.fork_note.unwrap().contains("Single agent"));
        assert_eq!(fx.finish(swapped).await.0, 200);
    })
}
