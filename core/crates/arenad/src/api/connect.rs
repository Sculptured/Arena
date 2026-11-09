//! `POST /api/agents/check`: the handshake behind "Test connection". It takes an `http` agent
//! (`{kind, url, headers?}`) or a Bedrock one (`{kind: "bedrock", model_id, region}`), reaches it
//! once, and answers `{ok, agent, name?, ms}` (the agent as a team stores it, its header values as
//! references) or `{ok: false, error, ms}`.

use std::{sync::Arc, time::Instant};

use axum::{body::Bytes, extract::State, routing::post, Json, Router};
use serde_json::{json, Value};

use super::{parse_body, validate_http, ApiError, ApiResult};
use crate::{
    agents::http::{hello, host_of},
    app::AppState,
    model::*,
    providers::BEDROCK_PREFIX,
};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/agents/check", post(check))
}

/// The agent the body describes, with an id and a name when it gives none.
fn agent_of(body: &Bytes) -> Result<AgentSpec, ApiError> {
    let mut v: Value = parse_body(body)?;
    let Some(o) = v.as_object_mut() else {
        return Err(ApiError::bad("bad request body: expected an object"));
    };
    o.entry("id").or_insert(json!("agent"));
    o.entry("name").or_insert(json!(""));
    serde_json::from_value(v).map_err(|e| ApiError::bad(format!("bad request body: {e}")))
}

fn is_bedrock(a: &AgentSpec) -> bool {
    a.kind == AgentKind::Open
        && a.provider
            .as_deref()
            .is_some_and(|p| p.starts_with(BEDROCK_PREFIX))
}

/// One Converse call of one token: the credentials, the region and the model all work.
async fn ping_bedrock(state: &AppState, a: &AgentSpec) -> Result<(), String> {
    let provider = state.config.providers.resolve(a.provider.as_deref())?;
    let Some(model) = a.model.as_deref().and_then(trimmed) else {
        return Err("name the Bedrock model id".into());
    };
    let call = json!({"model": model, "max_tokens": 1,
                      "messages": [{"role": "user", "content": "ping"}]});
    let u = provider.send(&state.http, &call).await?;
    if (200..300).contains(&u.status) {
        return Ok(());
    }
    let why = u.response["error"]["message"]
        .as_str()
        .unwrap_or("no message");
    Err(format!("Bedrock answered {}: {why}", u.status))
}

/// Reaches the agent once; the name it goes by.
async fn reach(state: &AppState, a: &AgentSpec) -> Result<Option<String>, String> {
    if is_bedrock(a) {
        return ping_bedrock(state, a).await.map(|()| None);
    }
    hello(&state.http, a).await
}

/// What an agent that names itself nothing is called: its host, or its model.
fn default_name(a: &AgentSpec) -> String {
    match a.kind {
        AgentKind::Http => host_of(a.url.as_deref().unwrap_or_default()),
        _ => a.model.clone().unwrap_or_default(),
    }
}

async fn check(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Value> {
    let mut agent = agent_of(&body)?;
    match agent.kind {
        AgentKind::Http => validate_http(&agent)?,
        _ if is_bedrock(&agent) => {}
        _ => {
            return Err(ApiError::bad(
                "the check takes an http agent or a bedrock agent",
            ))
        }
    }
    let started = Instant::now();
    let reached = reach(&state, &agent).await;
    let ms = started.elapsed().as_millis() as u64;
    let said = match reached {
        Ok(said) => said,
        Err(error) => return Ok(Json(json!({"ok": false, "error": error, "ms": ms}))),
    };
    if agent.name.trim().is_empty() {
        agent.name = said.clone().unwrap_or_else(|| default_name(&agent));
    }
    let mut answer = json!({"ok": true, "agent": agent, "ms": ms});
    if let Some(name) = said {
        answer["name"] = json!(name);
    }
    Ok(Json(answer))
}
