use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use crate::{
    app::AppState,
    model::*,
    providers::{Provider, Upstream},
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route(
        "/gateway/{branch}/{agent}/v1/chat/completions",
        post(handle),
    )
}

fn error_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

/// What the agent's provider answered: `Err(message)` when it can't be reached (or isn't known).
async fn send_upstream(
    state: &AppState,
    provider: &Result<Provider, String>,
    body: &Value,
) -> Result<Upstream, String> {
    match provider {
        Ok(p) => p.send(&state.http, body).await,
        Err(e) => Err(e.clone()),
    }
}

/// The status to answer with, the response body, and whether the call succeeded.
fn answer_for(upstream: &Result<Upstream, String>) -> (u16, Value, bool) {
    match upstream {
        Err(msg) => (502, json!({"error": msg}), false),
        Ok(u) if (200..300).contains(&u.status) => (u.status, u.response.clone(), true),
        Ok(u) => (
            502,
            json!({"error": format!("upstream returned {}", u.status), "status": u.status, "body": u.response}),
            false,
        ),
    }
}

/// The provider's own response, kept when it isn't the canonical one already recorded.
fn raw_evidence(upstream: &Result<Upstream, String>) -> Option<Value> {
    upstream
        .as_ref()
        .ok()
        .filter(|u| u.raw != u.response)
        .map(|u| u.raw.clone())
}

/// The event of one model call: the canonical request and response, the provider's own
/// response when it differs, and the cost the provider makes known.
fn model_call_payload(
    request: Value,
    provider: &Result<Provider, String>,
    upstream: &Result<Upstream, String>,
) -> (u16, ModelCallPayload) {
    let (status, response, ok) = answer_for(upstream);
    let usage = response.get("usage").cloned().unwrap_or(Value::Null);
    let known = provider.as_ref().ok().filter(|_| ok);
    let cost_usd = known.and_then(|p| p.cost(&usage));
    let payload = ModelCallPayload {
        request,
        usage,
        cost_source: cost_usd
            .and(known.and_then(Provider::cost_source))
            .map(str::to_string),
        cost_usd,
        provider: provider.as_ref().ok().map(|p| p.id.clone()),
        provider_response: raw_evidence(upstream),
        response,
    };
    (status, payload)
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

/// The provider of `agent` on the live `branch` (`Some(None)`: the default one); `None` when the
/// branch isn't live or has no such agent.
fn live_provider(state: &AppState, branch: &str, agent: &str) -> Option<Option<String>> {
    let live = state.live.read().unwrap();
    live.get(branch)?
        .provider_of(agent)
        .map(|p| p.map(str::to_string))
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let Some(provider) = live_provider(&state, &branch, &agent) else {
        return error_response(StatusCode::NOT_FOUND, "unknown branch or agent");
    };
    if !body.is_object() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object",
        );
    }
    let provider = state.config.providers.resolve(provider.as_deref());
    let upstream = send_upstream(&state, &provider, &body).await;
    let (status, payload) = model_call_payload(body, &provider, &upstream);
    if let Err(e) = record(&state, &branch, &agent, &payload).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording model call: {e}"),
        );
    }
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
        Json(payload.response),
    )
        .into_response()
}
