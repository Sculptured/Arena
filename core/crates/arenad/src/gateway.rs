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
    Router::new().route("/gateway/{branch}/{agent}/v1/chat/completions", post(handle))
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    Json(mut body): Json<Value>,
) -> Response {
    if !body.is_object() {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "request body must be a JSON object"}))).into_response();
    }
    body["usage"] = json!({"include": true});
    let cfg = &state.config;
    let url = format!("{}/chat/completions", cfg.upstream_base.trim_end_matches('/'));
    let mut req = state.http.post(&url).json(&body);
    if cfg.upstream_is_openrouter() {
        if let Some(key) = &cfg.openrouter_key {
            req = req.bearer_auth(key);
        }
    }
    // Err(message) for transport failures; Ok((status, json)) otherwise.
    let upstream: Result<(u16, Value), String> = match req.send().await {
        Err(e) => Err(format!("upstream request failed: {}", e.without_url())),
        Ok(r) => {
            let status = r.status().as_u16();
            match r.text().await {
                Err(e) => Err(format!("upstream response unreadable: {}", e.without_url())),
                Ok(t) => Ok((status, serde_json::from_str(&t).unwrap_or_else(|_| json!({"raw": t})))),
            }
        }
    };
    let (status, response, ok) = match upstream {
        Err(msg) => (502, json!({"error": msg}), false),
        Ok((s, v)) if (200..300).contains(&s) => (s, v, true),
        Ok((s, v)) => (502, json!({"error": format!("upstream returned {s}"), "status": s, "body": v}), false),
    };
    let usage = response.get("usage").cloned().unwrap_or(Value::Null);
    let cost_usd = if ok { usage.get("cost").and_then(Value::as_f64) } else { None };
    let payload = ModelCallPayload {
        request: body,
        response: response.clone(),
        usage,
        cost_source: cost_usd.map(|_| "openrouter".to_string()),
        cost_usd,
    };
    let recorded = match serde_json::to_value(&payload) {
        Ok(v) => state.store.append(&branch, Some(&agent), EventKind::ModelCall, v).await.map(|_| ()).map_err(|e| format!("{e:#}")),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = recorded {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": format!("recording model call: {e}")})))
            .into_response();
    }
    (StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY), Json(response)).into_response()
}
