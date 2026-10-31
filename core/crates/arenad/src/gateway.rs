use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use crate::{app::AppState, model::*};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/gateway/{branch}/{agent}/v1/chat/completions",
        post(handle),
    )
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

/// What the upstream answered: `Err(message)` for transport failures, `Ok((status, json))`
/// otherwise.
async fn send_upstream(state: &AppState, body: &Value) -> Result<(u16, Value), String> {
    let cfg = &state.config;
    let url = format!(
        "{}/chat/completions",
        cfg.upstream_base.trim_end_matches('/')
    );
    let mut req = state.http.post(&url).json(body);
    if cfg.upstream_is_openrouter() {
        if let Some(key) = &cfg.openrouter_key {
            req = req.bearer_auth(key);
        }
    }
    let r = req
        .send()
        .await
        .map_err(|e| format!("upstream request failed: {}", e.without_url()))?;
    let status = r.status().as_u16();
    let t = r
        .text()
        .await
        .map_err(|e| format!("upstream response unreadable: {}", e.without_url()))?;
    Ok((
        status,
        serde_json::from_str(&t).unwrap_or_else(|_| json!({"raw": t})),
    ))
}

/// The status to answer with, the response body, and whether the call succeeded.
fn answer_for(upstream: Result<(u16, Value), String>) -> (u16, Value, bool) {
    match upstream {
        Err(msg) => (502, json!({"error": msg}), false),
        Ok((s, v)) if (200..300).contains(&s) => (s, v, true),
        Ok((s, v)) => (
            502,
            json!({"error": format!("upstream returned {s}"), "status": s, "body": v}),
            false,
        ),
    }
}

fn model_call_payload(request: Value, response: &Value, ok: bool) -> ModelCallPayload {
    let usage = response.get("usage").cloned().unwrap_or(Value::Null);
    let cost_usd = if ok {
        usage.get("cost").and_then(Value::as_f64)
    } else {
        None
    };
    ModelCallPayload {
        request,
        response: response.clone(),
        usage,
        cost_source: cost_usd.map(|_| "openrouter".to_string()),
        cost_usd,
    }
}

async fn record(
    state: &AppState,
    branch: &str,
    agent: &str,
    payload: &ModelCallPayload,
) -> Result<(), String> {
    let v = serde_json::to_value(payload).map_err(|e| e.to_string())?;
    state
        .store
        .append(branch, Some(agent), EventKind::ModelCall, v)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

/// True when the branch is live and has the agent.
fn is_live(state: &AppState, branch: &str, agent: &str) -> bool {
    state
        .live
        .read()
        .unwrap()
        .get(branch)
        .is_some_and(|c| c.has_agent(agent))
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Response {
    if !is_live(&state, &branch, &agent) {
        return error_response(StatusCode::NOT_FOUND, "unknown branch or agent");
    }
    if !body.is_object() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object",
        );
    }
    body["usage"] = json!({"include": true});
    let (status, response, ok) = answer_for(send_upstream(&state, &body).await);
    let payload = model_call_payload(body, &response, ok);
    if let Err(e) = record(&state, &branch, &agent, &payload).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording model call: {e}"),
        );
    }
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(response),
    )
        .into_response()
}
