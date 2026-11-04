use std::{sync::Arc, time::Instant};

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
    exec::BranchCtx,
    model::*,
    overhead::{self, ms_since, Cat},
    providers::{Provider, Upstream},
    store::Store,
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
        Ok(p) => p.send(&state.http, body).await.map(redact_rejection),
        Err(e) => Err(e.clone()),
    }
}

/// What stands in for the message of an answer that turned the key down.
pub const REJECTED_MESSAGE: &str = "(not recorded: the provider rejected the key)";

/// A 401 or 403 answer without its error message, which may quote the key (masked); the
/// error's type and code stay.
fn redact_rejection(mut u: Upstream) -> Upstream {
    if matches!(u.status, 401 | 403) {
        redact_message(&mut u.response);
        redact_message(&mut u.raw);
    }
    u
}

fn redact_message(body: &mut Value) {
    match body.get_mut("error") {
        Some(Value::Object(e)) if e.contains_key("message") => {
            e.insert("message".into(), json!(REJECTED_MESSAGE));
        }
        Some(e @ Value::String(_)) => *e = json!(REJECTED_MESSAGE),
        _ => {}
    }
    // An answer that wasn't JSON is kept as `{"raw": text}`.
    if let Some(raw) = body.get_mut("raw") {
        *raw = json!(REJECTED_MESSAGE);
    }
}

/// The status to answer with, the response body, and whether the call succeeded. A provider's
/// error status is passed on (so the agent loop can tell overload and rate limits); no answer,
/// or an answer that is neither success nor error, is a 502.
fn answer_for(upstream: &Result<Upstream, String>) -> (u16, Value, bool) {
    match upstream {
        Err(msg) => (502, json!({"error": msg}), false),
        Ok(u) if (200..300).contains(&u.status) => (u.status, u.response.clone(), true),
        Ok(u) => (
            if (400..600).contains(&u.status) {
                u.status
            } else {
                502
            },
            json!({"error": format!("upstream returned {}", u.status), "status": u.status, "body": u.response}),
            false,
        ),
    }
}

/// The `retry-after` to pass on: what a provider that failed asked for.
fn retry_after(upstream: &Result<Upstream, String>) -> Option<u64> {
    upstream
        .as_ref()
        .ok()
        .filter(|u| !(200..300).contains(&u.status))
        .and_then(|u| u.retry_after)
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
    ctx: &BranchCtx,
    store: &Store,
    agent: &str,
    payload: &ModelCallPayload,
) -> Result<(), String> {
    let v = serde_json::to_value(payload).map_err(|e| e.to_string())?;
    ctx.append(store, Some(agent), EventKind::ModelCall, v)
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:#}"))
}

/// The live `branch` and the provider of its `agent` (`None`: the default one); `None` when the
/// branch isn't live or has no such agent.
fn live_agent(
    state: &AppState,
    branch: &str,
    agent: &str,
) -> Option<(Arc<BranchCtx>, Option<String>)> {
    let ctx = state.live.read().unwrap().get(branch)?.clone();
    let provider = ctx.provider_of(agent)?;
    Some((ctx, provider))
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Response {
    let since = Instant::now();
    let Some((ctx, provider)) = live_agent(&state, &branch, &agent) else {
        return error_response(StatusCode::NOT_FOUND, "unknown branch or agent");
    };
    if !body.is_object() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object",
        );
    }
    let provider = state.config.providers.resolve(provider.as_deref());
    let asked = Instant::now();
    let upstream = send_upstream(&state, &provider, &body).await;
    let upstream_ms = ms_since(asked);
    let (status, payload) = model_call_payload(body, &provider, &upstream);
    // The gateway's own handling: everything but the provider's answer (and the record's time,
    // which the store counts).
    overhead::add(&branch, Cat::Gateway, ms_since(since) - upstream_ms);
    if let Err(e) = record(&ctx, &state.store, &agent, &payload).await {
        return error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("recording model call: {e}"),
        );
    }
    reply(status, payload.response, &upstream)
}

/// The agent's answer: `status` and `body`, with the provider's `retry-after` when it failed.
fn reply(status: u16, body: Value, upstream: &Result<Upstream, String>) -> Response {
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response = (status, Json(body)).into_response();
    if let Some(secs) = retry_after(upstream) {
        response
            .headers_mut()
            .insert(axum::http::header::RETRY_AFTER, secs.into());
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(status: u16, body: Value) -> Upstream {
        redact_rejection(Upstream {
            status,
            response: body.clone(),
            raw: body,
            retry_after: None,
        })
    }

    #[test]
    fn a_rejected_keys_message_is_not_kept() {
        let openai = json!({"error": {"message": "Incorrect API key provided: sk-ab***yz",
                                      "type": "invalid_request_error", "code": "invalid_api_key"}});
        let u = answer(401, openai);
        let want = json!({"error": {"message": REJECTED_MESSAGE,
                                    "type": "invalid_request_error", "code": "invalid_api_key"}});
        assert_eq!((&u.response, &u.raw), (&want, &want));
        let claude = json!({"type": "error",
                            "error": {"type": "permission_error", "message": "key sk-ant-***"}});
        let u = answer(403, claude);
        assert_eq!(u.raw["error"]["type"], "permission_error");
        assert_eq!(u.raw["error"]["message"], REJECTED_MESSAGE);
        assert_eq!(
            answer(401, json!({"raw": "bad key sk-x"}).clone()).raw["raw"],
            REJECTED_MESSAGE
        );
        assert_eq!(
            answer(401, json!({"error": "bad key sk-x"})).response["error"],
            REJECTED_MESSAGE
        );
        // Other failures keep their message.
        let busy = json!({"error": {"message": "overloaded"}});
        assert_eq!(answer(529, busy.clone()).raw, busy);
    }
}
