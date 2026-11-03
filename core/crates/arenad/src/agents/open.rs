use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    time::Instant,
};

use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, FailureKind};
use crate::{
    app::AppState,
    exec::{tool_schemas, ToolExec},
    model::add_cost,
    providers::Provider,
    world::World,
};

pub const MAX_MODEL_CALLS: usize = 40;

/// How the loop ended badly: the model misbehaved, or something broke around it.
enum Fault {
    Model(String),
    Infra(String),
}

impl From<String> for Fault {
    fn from(s: String) -> Fault {
        Fault::Infra(s)
    }
}

impl From<&str> for Fault {
    fn from(s: &str) -> Fault {
        Fault::Infra(s.to_string())
    }
}

/// Our own agent loop: every model call goes through the recording gateway.
pub struct OpenDriver {
    // Weak: `AppState` owns the drivers, so a strong reference would be a cycle.
    state: Weak<AppState>,
}

impl OpenDriver {
    pub fn new(state: Arc<AppState>) -> OpenDriver {
        OpenDriver {
            state: Arc::downgrade(&state),
        }
    }
}

fn openai_tools(world: &World, allow_handoff: bool) -> Vec<Value> {
    tool_schemas(world, allow_handoff)
        .into_iter()
        .map(|s| {
            json!({"type": "function", "function": {
                "name": s["name"], "description": s["description"], "parameters": s["inputSchema"]}})
        })
        .collect()
}

fn initial_messages(run: &AgentRun) -> Vec<Value> {
    let system = json!({"role": "system", "content": run.spec.prompt});
    match &run.resume {
        None => vec![system, json!({"role": "user", "content": run.input})],
        Some(msgs) => {
            let mut out = vec![system];
            out.extend(msgs.iter().filter(|m| m["role"] != "system").cloned());
            out
        }
    }
}

impl Driver for OpenDriver {
    fn run<'a>(&'a self, run: AgentRun) -> Pin<Box<dyn Future<Output = AgentResult> + Send + 'a>> {
        Box::pin(self.run_inner(run))
    }
}

/// Token and cost totals over a run's model calls.
struct Totals {
    tokens: [i64; 3],
    /// `None` once a call's cost is unknown.
    cost: Option<f64>,
    /// The label of the cost: the provider's, once it is known.
    source: Option<&'static str>,
}

impl Totals {
    fn new() -> Totals {
        Totals {
            tokens: [0; 3],
            cost: Some(0.0),
            source: None,
        }
    }

    fn add(&mut self, provider: &Provider, usage: &Value) {
        for (total, k) in
            self.tokens
                .iter_mut()
                .zip(["prompt_tokens", "completion_tokens", "total_tokens"])
        {
            *total += usage[k].as_i64().unwrap_or(0);
        }
        self.cost = add_cost(self.cost, provider.cost(usage));
        self.source = provider.cost_source();
    }
}

/// The arguments of a tool call: `{}` for none, `None` when they are not a JSON object.
fn parse_tool_args(raw: &str) -> Option<Value> {
    if raw.trim().is_empty() {
        return Some(json!({}));
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
}

fn tool_message(id: &str, content: String) -> Value {
    json!({"role": "tool", "tool_call_id": id, "content": content})
}

/// One model call through the gateway; the parsed response.
async fn model_call(http: &reqwest::Client, url: &str, body: &Value) -> Result<Value, Fault> {
    let resp = http
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("gateway request failed: {e}"))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("gateway response unreadable: {e}"))?;
    if !status.is_success() {
        return Err(format!("model call failed ({status}): {text}").into());
    }
    Ok(serde_json::from_str(&text).map_err(|e| format!("bad model response: {e}"))?)
}

/// The recording gateway's address for this agent.
fn gateway_url(state: &AppState, run: &AgentRun) -> String {
    format!(
        "{}/gateway/{}/{}/v1/chat/completions",
        state.config.self_base.trim_end_matches('/'),
        run.branch.branch_id,
        run.spec.id
    )
}

impl OpenDriver {
    async fn run_inner(&self, run: AgentRun) -> AgentResult {
        let start = Instant::now();
        let mut totals = Totals::new();
        let outcome = self.drive(&run, &mut totals).await;
        let (error, failure) = match outcome {
            Ok(()) => (None, None),
            Err(Fault::Model(e)) => (Some(e), Some(FailureKind::Model)),
            Err(Fault::Infra(e)) => (Some(e), None),
        };
        let [prompt, completion, total] = totals.tokens;
        AgentResult {
            ok: error.is_none(),
            error,
            failure,
            tokens: super::tokens_json(prompt, completion, total),
            cost_source: totals.cost.and(totals.source).map(str::to_string),
            cost_usd: totals.cost,
            seconds: start.elapsed().as_secs_f64(),
        }
    }

    async fn drive(&self, run: &AgentRun, totals: &mut Totals) -> Result<(), Fault> {
        let state = self.state.upgrade().ok_or("server is shutting down")?;
        let (provider, model) = provider_and_model(&state, run)?;
        let url = gateway_url(&state, run);
        let tools = openai_tools(&run.branch.world, !run.branch.is_last(&run.spec.id));
        let mut messages = initial_messages(run);
        let exec = ToolExec::new(state.store.clone());
        for _ in 0..MAX_MODEL_CALLS {
            let body = json!({"messages": messages, "tools": tools, "model": model});
            let v = model_call(&state.http, &url, &body).await?;
            totals.add(&provider, &v["usage"]);
            let msg = v["choices"][0]["message"].clone();
            if !msg.is_object() {
                return Err("model response has no message".into());
            }
            messages.push(msg.clone());
            let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
            if calls.is_empty() {
                return end_without_calls(&v);
            }
            if run_calls(&exec, run, calls, &mut messages).await? {
                return Ok(());
            }
        }
        Err(Fault::Model(format!(
            "agent exceeded {MAX_MODEL_CALLS} model calls"
        )))
    }
}

/// The agent's provider, and its model, else the provider's default; an error when the
/// provider is unknown or has no default.
fn provider_and_model<'a>(
    state: &AppState,
    run: &'a AgentRun,
) -> Result<(Provider, &'a str), Fault> {
    let provider = state
        .config
        .providers
        .resolve(run.spec.provider.as_deref())?;
    let named = run.spec.model.as_deref().filter(|m| !m.trim().is_empty());
    let model = named.or(provider.default_model()).ok_or_else(|| {
        Fault::Infra(format!(
            "agent {} names no model, and provider {} has no default one",
            run.spec.id, provider.id
        ))
    })?;
    Ok((provider, model))
}

/// The model answered with no tool calls: done, unless its output was cut off.
fn end_without_calls(response: &Value) -> Result<(), Fault> {
    if response["choices"][0]["finish_reason"] == "length" {
        return Err(Fault::Model("model output was cut off".into()));
    }
    Ok(())
}

/// Runs each tool call, appending the tool messages. `Ok(true)` when the agent handed off.
async fn run_calls(
    exec: &ToolExec,
    run: &AgentRun,
    calls: Vec<Value>,
    messages: &mut Vec<Value>,
) -> Result<bool, Fault> {
    let agent = run.spec.id.as_str();
    for call in calls {
        let id = call["id"].as_str().unwrap_or("").to_string();
        let name = call["function"]["name"].as_str().unwrap_or("").to_string();
        let raw = call["function"]["arguments"].as_str().unwrap_or("");
        let Some(args) = parse_tool_args(raw) else {
            let content = json!({"error": format!(
                "arguments for {name} were not valid JSON (the output may have been cut off); the tool was not run")});
            messages.push(tool_message(&id, content.to_string()));
            continue;
        };
        let res = exec
            .run_outcome(&run.branch, agent, &name, args, Some(id.clone()))
            .await;
        let content = match &res {
            Ok(v) => v.to_string(),
            Err(e) => json!({"error": e}).to_string(),
        };
        messages.push(tool_message(&id, content));
        if name == "handoff" && res.is_ok() {
            return Ok(true);
        }
        // A world program stopped: the branch can't go on.
        if let Some(f) = run.branch.fatal() {
            return Err(Fault::Infra(f));
        }
    }
    Ok(false)
}
