use std::{future::Future, pin::Pin, sync::Arc, time::Instant};

use serde_json::{json, Value};
use world::tools;

use super::{AgentResult, AgentRun, Driver};
use crate::{
    app::AppState,
    exec::{handoff_schema, ToolExec},
};

pub const MAX_MODEL_CALLS: usize = 40;

/// Our own agent loop: every model call goes through the recording gateway.
pub struct OpenDriver {
    state: Arc<AppState>,
}

impl OpenDriver {
    pub fn new(state: Arc<AppState>) -> OpenDriver {
        OpenDriver { state }
    }
}

fn openai_tools(allow_handoff: bool) -> Vec<Value> {
    let mut schemas = tools::tool_schemas();
    if allow_handoff {
        schemas.push(handoff_schema());
    }
    schemas
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

impl OpenDriver {
    async fn run_inner(&self, run: AgentRun) -> AgentResult {
        let start = Instant::now();
        let mut totals = [0i64; 3];
        let mut cost = Some(0.0f64);
        let outcome = self.drive(&run, &mut totals, &mut cost).await;
        AgentResult {
            ok: outcome.is_ok(),
            error: outcome.err(),
            tokens: json!({"prompt_tokens": totals[0], "completion_tokens": totals[1], "total_tokens": totals[2]}),
            cost_source: cost.map(|_| "openrouter".to_string()),
            cost_usd: cost,
            seconds: start.elapsed().as_secs_f64(),
        }
    }

    async fn drive(&self, run: &AgentRun, totals: &mut [i64; 3], cost: &mut Option<f64>) -> Result<(), String> {
        let agent = run.spec.id.as_str();
        let url = format!(
            "{}/gateway/{}/{}/v1/chat/completions",
            self.state.config.self_base.trim_end_matches('/'),
            run.branch.branch_id,
            agent
        );
        let tools = openai_tools(!run.branch.is_last(agent));
        let mut messages = initial_messages(run);
        let exec = ToolExec::new(self.state.store.clone(), self.state.worlds.clone());
        let http = &self.state.http;
        for _ in 0..MAX_MODEL_CALLS {
            let mut body = json!({"messages": messages, "tools": tools});
            if let Some(m) = &run.spec.model {
                body["model"] = json!(m);
            }
            let resp = http.post(&url).json(&body).send().await.map_err(|e| format!("gateway request failed: {e}"))?;
            let status = resp.status();
            let text = resp.text().await.map_err(|e| format!("gateway response unreadable: {e}"))?;
            if !status.is_success() {
                return Err(format!("model call failed ({status}): {text}"));
            }
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("bad model response: {e}"))?;
            let usage = &v["usage"];
            for (i, k) in ["prompt_tokens", "completion_tokens", "total_tokens"].iter().enumerate() {
                totals[i] += usage[k].as_i64().unwrap_or(0);
            }
            *cost = match (*cost, usage["cost"].as_f64()) {
                (Some(a), Some(b)) => Some(a + b),
                _ => None,
            };
            let msg = v["choices"][0]["message"].clone();
            if !msg.is_object() {
                return Err("model response has no message".into());
            }
            messages.push(msg.clone());
            let calls = msg["tool_calls"].as_array().cloned().unwrap_or_default();
            if calls.is_empty() {
                if v["choices"][0]["finish_reason"] == "length" {
                    return Err("model output was cut off".into());
                }
                return Ok(());
            }
            for call in calls {
                let id = call["id"].as_str().unwrap_or("").to_string();
                let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                let raw = call["function"]["arguments"].as_str().unwrap_or("");
                let parsed = if raw.trim().is_empty() {
                    Some(json!({}))
                } else {
                    serde_json::from_str::<Value>(raw).ok().filter(Value::is_object)
                };
                let Some(args) = parsed else {
                    let content = json!({"error": format!(
                        "arguments for {name} were not valid JSON (the output may have been cut off); the tool was not run")});
                    messages.push(json!({"role": "tool", "tool_call_id": id, "content": content.to_string()}));
                    continue;
                };
                let res = exec.run_outcome(&run.branch, agent, &name, args, Some(id.clone())).await;
                let content = match &res {
                    Ok(v) => v.to_string(),
                    Err(e) => json!({"error": e}).to_string(),
                };
                messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
                if name == "handoff" && res.is_ok() {
                    return Ok(());
                }
            }
        }
        Err(format!("agent exceeded {MAX_MODEL_CALLS} model calls"))
    }
}
