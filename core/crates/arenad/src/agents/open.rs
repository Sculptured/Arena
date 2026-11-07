use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use serde_json::{json, Value};

use super::{AgentResult, AgentRun, Driver, FailureKind};
use crate::{
    app::AppState,
    exec::{tool_schemas, BranchCtx, ToolExec},
    model::add_cost,
    providers::Provider,
};

pub const MAX_MODEL_CALLS: usize = 40;
/// How many times a model call is tried again when its provider is overloaded or
/// rate-limiting (statuses 429, 503 and 529); every try is recorded.
pub const MAX_RETRIES: u32 = 3;
const RETRYABLE: [u16; 3] = [429, 503, 529];
/// The longest wait before a retry, whatever the provider asks.
pub const MAX_RETRY_WAIT: Duration = Duration::from_secs(20);

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

fn openai_tools(branch: &BranchCtx, agent: &str) -> Vec<Value> {
    tool_schemas(branch, agent)
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
    /// The label of the cost: the provider's, once the provider is known.
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

/// What the gateway answered: its status, the `retry-after` seconds it passed on, its body.
struct Answer {
    status: reqwest::StatusCode,
    retry_after: Option<u64>,
    text: String,
}

async fn post(http: &reqwest::Client, url: &str, body: &Value) -> Result<Answer, Fault> {
    let resp = http
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|e| format!("gateway request failed: {e}"))?;
    let status = resp.status();
    let retry_after = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok()?.parse().ok());
    let text = resp
        .text()
        .await
        .map_err(|e| format!("gateway response unreadable: {e}"))?;
    Ok(Answer {
        status,
        retry_after,
        text,
    })
}

/// One model call through the gateway, tried again (up to [`MAX_RETRIES`] times, after
/// [`retry_wait`]) while the provider is overloaded or rate-limiting; the parsed response.
async fn model_call(
    http: &reqwest::Client,
    url: &str,
    body: &Value,
    retry_base: Duration,
) -> Result<Value, Fault> {
    let mut retries = 0;
    loop {
        let a = post(http, url, body).await?;
        if a.status.is_success() {
            return Ok(
                serde_json::from_str(&a.text).map_err(|e| format!("bad model response: {e}"))?
            );
        }
        if retries == MAX_RETRIES || !RETRYABLE.contains(&a.status.as_u16()) {
            return Err(failure(a.status, &a.text).into());
        }
        tokio::time::sleep(retry_wait(retry_base, retries, a.retry_after)).await;
        retries += 1;
    }
}

/// The wait before retry number `retries + 1`: what the provider asked, else `base` doubled
/// for each retry already made; at most [`MAX_RETRY_WAIT`].
fn retry_wait(base: Duration, retries: u32, asked: Option<u64>) -> Duration {
    let wait = asked.map_or_else(|| base.saturating_mul(1 << retries), Duration::from_secs);
    wait.min(MAX_RETRY_WAIT)
}

/// Why a model call failed, in plain words: the provider's own error message when it gave one.
fn failure(status: reqwest::StatusCode, text: &str) -> String {
    let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let error = &v["body"]["error"];
    let detail = [&error["message"], error, &v["error"]]
        .into_iter()
        .find_map(Value::as_str)
        .unwrap_or(text);
    format!("model call failed ({}): {detail}", status.as_u16())
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
        totals.source = provider.cost_source();
        let url = gateway_url(&state, run);
        let tools = openai_tools(&run.branch, &run.spec.id);
        let mut messages = initial_messages(run);
        let exec = ToolExec::new(state.store.clone());
        for _ in 0..MAX_MODEL_CALLS {
            let body = json!({"messages": messages, "tools": tools, "model": model});
            let v = model_call(&state.http, &url, &body, state.config.retry_base).await?;
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

/// The model answered with no tool calls: done, unless its output was cut off or it refused
/// (`content_filter`, with the category from Claude's `stop_details` when there is one).
fn end_without_calls(response: &Value) -> Result<(), Fault> {
    let choice = &response["choices"][0];
    match choice["finish_reason"].as_str() {
        Some("length") => Err(Fault::Model("model output was cut off".into())),
        Some("content_filter") => Err(Fault::Model(refusal(&choice["stop_details"]))),
        _ => Ok(()),
    }
}

fn refusal(details: &Value) -> String {
    match details["category"].as_str() {
        Some(category) => format!("the model refused (category {category})"),
        None => "the model refused".to_string(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_wait_as_asked_else_twice_as_long_each_time() {
        let base = Duration::from_secs(1);
        let waits: Vec<u64> = (0..3)
            .map(|n| retry_wait(base, n, None).as_secs())
            .collect();
        assert_eq!(waits, [1, 2, 4]);
        assert_eq!(retry_wait(base, 2, Some(0)), Duration::ZERO);
        assert_eq!(retry_wait(base, 0, Some(7)).as_secs(), 7);
        assert_eq!(retry_wait(base, 0, Some(600)), MAX_RETRY_WAIT);
        assert_eq!(retry_wait(base, 30, None), MAX_RETRY_WAIT);
    }

    /// Arguments cut off mid-JSON (or not an object) are not run; none at all are `{}`.
    #[test]
    fn only_a_json_object_is_run_as_arguments() {
        assert_eq!(parse_tool_args(""), Some(json!({})));
        assert_eq!(
            parse_tool_args(r##"{"order_id": "#W1"}"##),
            Some(json!({"order_id": "#W1"}))
        );
        assert_eq!(parse_tool_args(r##"{"order_id": "#W00"##), None);
        assert_eq!(parse_tool_args("[1]"), None);
    }

    /// An answer without tool calls ends the agent well, unless its output was cut off or the
    /// model refused: both are the model's failures, not the infrastructure's.
    #[test]
    fn a_cut_off_or_refused_answer_is_a_model_failure() {
        let answer = |finish: &str, details: Value| {
            json!({"choices": [{"message": {"role": "assistant", "content": "x"},
                                "finish_reason": finish, "stop_details": details}]})
        };
        assert!(end_without_calls(&answer("stop", Value::Null)).is_ok());
        let model = |r: Result<(), Fault>| match r {
            Err(Fault::Model(e)) => e,
            _ => panic!("not a model failure"),
        };
        assert_eq!(
            model(end_without_calls(&answer("length", Value::Null))),
            "model output was cut off"
        );
        let refused = answer(
            "content_filter",
            json!({"type": "refusal", "category": "cyber"}),
        );
        assert_eq!(
            model(end_without_calls(&refused)),
            "the model refused (category cyber)"
        );
        assert_eq!(
            model(end_without_calls(&answer("content_filter", Value::Null))),
            "the model refused"
        );
    }

    #[test]
    fn failures_say_what_the_provider_said() {
        let claude = json!({"error": "upstream returned 529", "status": 529, "body":
            {"type": "error", "error": {"type": "overloaded_error", "message": "Overloaded"}}});
        let status = reqwest::StatusCode::from_u16(529).unwrap();
        assert_eq!(
            failure(status, &claude.to_string()),
            "model call failed (529): Overloaded"
        );
        let unknown = json!({"error": "unknown provider \"x\""}).to_string();
        let bad = reqwest::StatusCode::BAD_GATEWAY;
        assert_eq!(
            failure(bad, &unknown),
            "model call failed (502): unknown provider \"x\""
        );
        assert_eq!(
            failure(bad, "not json"),
            "model call failed (502): not json"
        );
    }
}
