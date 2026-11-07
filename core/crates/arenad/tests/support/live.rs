//! A real model for the tests that run agents. Arena tests agent behaviour on real models only,
//! never on a fake model server or a scripted agent: a model's answers vary from run to run, so
//! these tests assert what Arena records and enforces (the run starts and ends, every step is
//! recorded with its fields, a gate refuses what it must when such a call happens, the checks are
//! computed, a fork restores exactly), never a model's wording or one path through a task.
//!
//! Every such test is named `live_…` and `#[ignore]`d. When no model is available it prints
//! `skipped: …` and passes. Which model it runs on:
//!
//! - Claude Code (`claude` on `PATH`, signed in), with `ARENA_LIVE_MODEL` (default `haiku`, the
//!   cheapest). Its tool calls are recorded through arenad's MCP endpoint; its model calls are
//!   its own, so a test that needs recorded model calls skips it.
//! - arenad's own agent loop on an API provider, when its key is set: `ANTHROPIC_API_KEY`
//!   (`claude-haiku-4-5`), `OPENAI_API_KEY` (`gpt-4.1-mini`), `OPENROUTER_API_KEY`
//!   (`anthropic/claude-haiku-4.5`), or the first server in `ARENA_LOCAL_MODELS` (name its model
//!   in `ARENA_LIVE_MODEL`). Every model call then goes through arenad's recording gateway.
//!
//! `ARENA_LIVE_AGENT=claude` or `=api` picks one; unset, Claude Code is used when it is installed,
//! else the API provider. Run them one at a time (they start worlds and agents):
//! `cargo test -p arenad -- --ignored live_ --test-threads 1 --nocapture`.
#![allow(dead_code)]

use std::{process::Command, sync::Arc, time::Duration};

use arenad::{app::AppState, model::*};
use serde_json::{json, Value};

/// The model a live test's agents run on.
#[derive(Debug, Clone, PartialEq)]
pub enum Live {
    /// Claude Code, signed in, with this model.
    Claude { model: String },
    /// arenad's own agent loop on `provider` with `model`.
    Api { provider: String, model: String },
}

/// The model `ARENA_LIVE_MODEL` names, else `default`.
fn model_or(default: &str) -> String {
    std::env::var("ARENA_LIVE_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
}

fn claude_installed() -> bool {
    Command::new("claude")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The API provider whose key is set (or the first local server), and its cheap model.
fn api_provider() -> Option<Live> {
    let api = |provider: &str, model: &str| Live::Api {
        provider: provider.into(),
        model: model_or(model),
    };
    if set("ANTHROPIC_API_KEY") {
        return Some(api("anthropic", "claude-haiku-4-5"));
    }
    if set("OPENAI_API_KEY") {
        return Some(api("openai", "gpt-4.1-mini"));
    }
    if set("OPENROUTER_API_KEY") {
        return Some(api("openrouter", "anthropic/claude-haiku-4.5"));
    }
    let locals = std::env::var(arenad::providers::LOCAL_MODELS_ENV).unwrap_or_default();
    let first = locals.split(',').next().and_then(|s| s.split_once('='));
    let name = first.map(|(n, _)| n.trim().to_string());
    match (name, std::env::var("ARENA_LIVE_MODEL")) {
        (Some(n), Ok(m)) if !n.is_empty() && !m.trim().is_empty() => Some(Live::Api {
            provider: format!("local:{n}"),
            model: m,
        }),
        _ => None,
    }
}

fn skip(test: &str, why: &str) {
    eprintln!("skipped: {test}: {why}");
}

impl Live {
    /// The model to run `test`'s agents on, or `None` (said why) when there is none.
    pub fn any(test: &str) -> Option<Live> {
        let choice = std::env::var("ARENA_LIVE_AGENT").unwrap_or_default();
        let claude = || {
            claude_installed().then(|| Live::Claude {
                model: model_or("haiku"),
            })
        };
        let found = match choice.as_str() {
            "claude" => claude(),
            "api" => api_provider(),
            _ => claude().or_else(api_provider),
        };
        if found.is_none() {
            skip(test, "no model: install and sign in to Claude Code, or set an API key (see support/live.rs)");
        }
        found
    }

    /// Claude Code, for a test that runs it itself (as an external system would), or `None`
    /// (said why).
    pub fn claude(test: &str) -> Option<Live> {
        let found = claude_installed().then(|| Live::Claude {
            model: model_or("haiku"),
        });
        if found.is_none() {
            skip(test, "needs Claude Code (`claude` on PATH, signed in)");
        }
        found
    }

    /// An API provider for a test that needs arenad's own agent loop (its model calls recorded
    /// by the gateway, a fork resuming mid-conversation), or `None` (said why).
    pub fn api(test: &str) -> Option<Live> {
        let found = api_provider();
        if found.is_none() {
            skip(test, "needs arenad's own agent loop: set ANTHROPIC_API_KEY, OPENAI_API_KEY, OPENROUTER_API_KEY or ARENA_LOCAL_MODELS with ARENA_LIVE_MODEL");
        }
        found
    }

    pub fn kind(&self) -> AgentKind {
        match self {
            Live::Claude { .. } => AgentKind::ClaudeCode,
            Live::Api { .. } => AgentKind::Open,
        }
    }

    pub fn model(&self) -> &str {
        match self {
            Live::Claude { model } | Live::Api { model, .. } => model,
        }
    }

    /// The provider an open agent names (`None` for Claude Code).
    pub fn provider(&self) -> Option<&str> {
        match self {
            Live::Claude { .. } => None,
            Live::Api { provider, .. } => Some(provider),
        }
    }

    /// True when arenad records each model call itself (its own loop through the gateway).
    pub fn records_model_calls(&self) -> bool {
        matches!(self, Live::Api { .. })
    }

    /// An agent `id` on this model with `prompt`.
    pub fn agent(&self, id: &str, prompt: &str) -> AgentSpec {
        AgentSpec {
            id: id.into(),
            name: id.into(),
            kind: self.kind(),
            model: Some(self.model().into()),
            prompt: prompt.into(),
            provider: self.provider().map(str::to_string),
            read_only: false,
        }
    }

    /// `agent`, but offered no write tool.
    pub fn reader(&self, id: &str, prompt: &str) -> AgentSpec {
        AgentSpec {
            read_only: true,
            ..self.agent(id, prompt)
        }
    }

    /// A team of `agents`.
    pub fn team(&self, agents: Vec<AgentSpec>) -> Team {
        Team {
            id: "live".into(),
            name: "Live".into(),
            agents,
        }
    }

    /// `hubteam`, a benchmark's own team, on this model: each role keeps its id, name, prompt
    /// and `read_only`.
    pub fn bench_team(&self, hubteam: parts::proto::BenchTeam) -> Team {
        let agent = |a: parts::proto::TeamRole| AgentSpec {
            read_only: a.read_only,
            name: a.name,
            ..self.agent(&a.id, &a.prompt)
        };
        Team {
            id: hubteam.id,
            name: hubteam.name,
            agents: hubteam.agents.into_iter().map(agent).collect(),
        }
    }

    /// `team` as the JSON `POST /api/runs` takes.
    pub fn team_json(&self, team: &Team) -> Value {
        serde_json::to_value(team).unwrap()
    }
}

/// Installs the real drivers (Claude Code, Codex, arenad's own loop) on `state`, whose
/// `self_base` must be final.
pub fn install(state: &Arc<AppState>) {
    assert!(arenad::app::install_default_drivers(state));
}

/// How long a live branch may run before the test gives up.
pub const BRANCH_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// The branch once it has ended (any status but `running`).
pub async fn ended(state: &AppState, branch: &str) -> Branch {
    let polls = BRANCH_TIMEOUT.as_millis() / 500;
    for _ in 0..polls {
        let b = state.store.branch(branch).await.unwrap();
        if b.status != BranchStatus::Running {
            return b;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("branch {branch} still running after {BRANCH_TIMEOUT:?}");
}

/// The branch once it has ended, asserting it ended for a reason of the task (passed or failed,
/// not an infrastructure error), and its events.
pub async fn ended_ok(state: &AppState, branch: &str) -> (Branch, Vec<Event>) {
    let b = ended(state, branch).await;
    assert_ne!(b.status, BranchStatus::InfraError, "{:?}", b.error);
    assert_ne!(b.status, BranchStatus::Interrupted, "{:?}", b.error);
    let evs = state.store.events(branch).await.unwrap();
    (b, evs)
}

/// The tool calls of `evs` made by `actor` (an agent id, `setup` or `link`).
pub fn calls_by<'a>(evs: &'a [Event], actor: &str) -> Vec<&'a Event> {
    evs.iter()
        .filter(|e| e.kind == EventKind::ToolCall && e.agent.as_deref() == Some(actor))
        .collect()
}

/// The tool calls named `name`, by anyone.
pub fn calls_named<'a>(evs: &'a [Event], name: &str) -> Vec<&'a Event> {
    evs.iter()
        .filter(|e| e.kind == EventKind::ToolCall && e.payload["name"] == name)
        .collect()
}

/// The seq of the first `kind` event of `agent`.
pub fn first_of(evs: &[Event], kind: EventKind, agent: &str) -> Option<i64> {
    evs.iter()
        .find(|e| e.kind == kind && e.agent.as_deref() == Some(agent))
        .map(|e| e.seq)
}

/// The branch's `Check` payload, if it has one.
pub fn check_of(evs: &[Event]) -> Option<CheckPayload> {
    let check = evs.iter().rev().find(|e| e.kind == EventKind::Check)?;
    Some(serde_json::from_value(check.payload.clone()).unwrap())
}

/// What every live branch's record must hold, whatever the model did: steps numbered from 1
/// without gaps; each agent that started ended, once, after its start; each tool call recorded
/// with its name, arguments and either a result or an error, by an agent of the team (or the
/// world's `setup` and `link`); a write that succeeded saved by every part that can save; each
/// model call (arenad's own loop) with the agent's model and provider and its usage; and a
/// branch that passed or failed for the task has its check.
pub fn assert_recorded(b: &Branch, evs: &[Event], live: &Live) {
    let seqs: Vec<i64> = evs.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=evs.len() as i64).collect::<Vec<_>>(), "steps numbered without gaps");
    assert_eq!(evs[0].kind, EventKind::RunStart, "{:?}", evs[0]);
    let ids: Vec<&str> = b.team.agents.iter().map(|a| a.id.as_str()).collect();
    for e in evs {
        match e.kind {
            EventKind::ToolCall => assert_tool_call(e, &ids),
            EventKind::ModelCall if live.records_model_calls() => assert_model_call(e, b),
            _ => {}
        }
    }
    for a in &b.team.agents {
        // An external system may leave an agent it registered without ending it.
        let must_end = a.kind != AgentKind::External;
        assert_started_and_ended(evs, &a.id, live, must_end);
    }
    let checked = check_of(evs).is_some();
    if matches!(b.status, BranchStatus::Passed) {
        assert!(checked, "a passed branch has its check");
    }
    let count = |k: EventKind| evs.iter().filter(|e| e.kind == k).count();
    eprintln!(
        "live: {} on {live:?}: {:?}, {} tool calls, {} model calls recorded, {} handoffs, {} messages",
        b.id,
        b.status,
        count(EventKind::ToolCall),
        count(EventKind::ModelCall),
        count(EventKind::Handoff),
        count(EventKind::Message)
    );
}

fn assert_tool_call(e: &Event, ids: &[&str]) {
    let p: ToolCallPayload = serde_json::from_value(e.payload.clone()).unwrap();
    let actor = e.agent.as_deref().unwrap_or_default();
    assert!(
        ids.contains(&actor) || actor == "setup" || actor == "link",
        "a tool call by {actor:?}"
    );
    assert!(!p.name.is_empty(), "{}", e.payload);
    assert!(p.args.is_object(), "{}", e.payload);
    if p.error.is_none() {
        assert!(e.payload.get("result").is_some(), "{}", e.payload);
    }
    assert!(p.duration_ms.is_none_or(|d| d >= 0.0), "{}", e.payload);
}

fn assert_model_call(e: &Event, b: &Branch) {
    let mc: ModelCallPayload = serde_json::from_value(e.payload.clone()).unwrap();
    let id = e.agent.as_deref().unwrap_or_default();
    let spec = b.team.agents.iter().find(|a| a.id == id);
    let spec = spec.unwrap_or_else(|| panic!("a model call by {id:?}"));
    assert_eq!(mc.request["model"].as_str(), spec.model.as_deref(), "{id}");
    assert!(mc.request["messages"].as_array().is_some_and(|m| !m.is_empty()), "{id}");
    assert_eq!(mc.provider.as_deref(), spec.provider.as_deref(), "{id}");
    if mc.response["choices"].is_array() {
        let prompt = mc.usage["prompt_tokens"].as_i64().unwrap_or(0);
        assert!(prompt > 0, "a call's usage: {}", mc.usage);
    }
}

/// `id` ended at most as often as it started, each end after a start (and as often, when it
/// `must_end`); an agent that ended well on Claude Code reports the tokens it used.
fn assert_started_and_ended(evs: &[Event], id: &str, live: &Live, must_end: bool) {
    let mut open = 0;
    for e in evs.iter().filter(|e| e.agent.as_deref() == Some(id)) {
        match e.kind {
            EventKind::AgentStart => open += 1,
            EventKind::AgentEnd => {
                assert!(open > 0, "{id} ended before it started");
                open -= 1;
                let end: AgentEndPayload = serde_json::from_value(e.payload.clone()).unwrap();
                assert!(end.seconds.is_none_or(|s| s >= 0.0), "{}", e.payload);
                if end.ok && matches!(live, Live::Claude { .. }) {
                    assert!(end.tokens.unwrap_or(0) > 0, "{id}'s end: {}", e.payload);
                }
            }
            _ => {}
        }
    }
    if must_end {
        assert_eq!(open, 0, "{id} started without ending");
    }
}

/// The tools `team`'s agents may call, from the first model call of an open agent (or `None`
/// when arenad doesn't see the model's requests).
pub fn offered_tools(evs: &[Event], agent: &str) -> Option<Vec<String>> {
    let mc = evs
        .iter()
        .find(|e| e.kind == EventKind::ModelCall && e.agent.as_deref() == Some(agent))?;
    let tools = mc.payload["request"]["tools"].as_array()?;
    let names = tools.iter().filter_map(|t| t["function"]["name"].as_str());
    Some(names.map(str::to_string).collect())
}

/// A JSON-RPC `tools/list` on an agent's MCP endpoint: the names of the tools it offers.
pub async fn mcp_tools(url: &str) -> Vec<String> {
    let req = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    let r = reqwest::Client::new().post(url).json(&req).send().await.unwrap();
    let v: Value = r.json().await.unwrap();
    let tools = v["result"]["tools"].as_array().cloned().unwrap_or_default();
    tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

/// What one Claude Code run an external system made reported: each model call its stream
/// showed (once per message), its final `result` message, and the tokens it used.
pub struct ExternalRun {
    pub reports: Vec<Value>,
    pub result: Value,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
}

/// One `usage` report per assistant message of a Claude Code `stream-json` output (the stream
/// repeats a message once per content block), in Arena's `POST .../usage` shape.
fn usage_reports(stdout: &str) -> (Vec<Value>, Value) {
    let (mut reports, mut seen, mut result) = (vec![], vec![], Value::Null);
    for line in stdout.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["type"] == "result" {
            result = v;
            continue;
        }
        let m = &v["message"];
        let id = m["id"].as_str().unwrap_or_default().to_string();
        if v["type"] != "assistant" || id.is_empty() || seen.contains(&id) {
            continue;
        }
        seen.push(id.clone());
        let u = &m["usage"];
        let n = |k: &str| u[k].as_i64().unwrap_or(0);
        reports.push(json!({"message_id": id, "model": m["model"],
            "input_tokens": n("input_tokens"), "output_tokens": n("output_tokens"),
            "cache_creation_input_tokens": n("cache_creation_input_tokens"),
            "cache_read_input_tokens": n("cache_read_input_tokens")}));
    }
    (reports, result)
}

/// Runs Claude Code (`live` must be [`Live::Claude`]) on `prompt` with the world's tools at
/// `mcp_url`, the way an external system runs its own agents: no built-in tools, only the MCP
/// server `server`'s, pre-approved. Ten minutes at most.
pub async fn run_claude(live: &Live, server: &str, mcp_url: &str, prompt: &str) -> ExternalRun {
    let config = json!({"mcpServers": {server: {"type": "http", "url": mcp_url}}});
    let workdir = std::env::temp_dir().join(format!("arena-live-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&workdir).unwrap();
    let mut c = tokio::process::Command::new("claude");
    c.current_dir(&workdir)
        .arg("-p")
        .args(["--model", live.model()])
        .args(["--output-format", "stream-json", "--verbose"])
        .args(["--setting-sources", ""])
        .arg("--strict-mcp-config")
        .arg("--mcp-config")
        .arg(config.to_string())
        .arg("--disable-slash-commands")
        .args(["--tools", ""])
        .arg("--allowedTools")
        .arg(format!("mcp__{server}__*"))
        .arg("--no-session-persistence")
        .args(["--", prompt])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for k in ["ARENA_PG_URL", "DATABASE_URL", "PGPASSWORD"] {
        c.env_remove(k);
    }
    let out = tokio::time::timeout(Duration::from_secs(600), c.output()).await;
    let _ = std::fs::remove_dir_all(&workdir);
    let out = out.expect("claude ran over 10 minutes").expect("claude starts");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (reports, result) = usage_reports(&stdout);
    let total = |k: &str| reports.iter().map(|r| r[k].as_i64().unwrap_or(0)).sum::<i64>();
    let prompt_tokens = total("input_tokens")
        + total("cache_creation_input_tokens")
        + total("cache_read_input_tokens");
    let completion_tokens = result["usage"]["output_tokens"].as_i64().unwrap_or(total("output_tokens"));
    ExternalRun {
        reports,
        result,
        prompt_tokens,
        completion_tokens,
    }
}
