//! A scripted OpenAI-compatible upstream for tests. Serves the script's assistant
//! messages in order (repeating the last one when exhausted) with fixed usage, and
//! records every request body it received. A script entry of the form
//! `{"_status": 500}` makes that call fail with that HTTP status; a message with
//! `_finish_reason` is served with that finish_reason (the key is removed).
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde_json::{json, Value};

pub struct FakeUpstream {
    pub base_url: String,
    state: Arc<St>,
}

struct St {
    script: Vec<Value>,
    seen: Mutex<Vec<Value>>,
    headers: Mutex<Vec<Option<String>>>,
}

impl FakeUpstream {
    pub async fn start(script: Vec<Value>) -> FakeUpstream {
        let state = Arc::new(St { script, seen: Mutex::new(vec![]), headers: Mutex::new(vec![]) });
        let app = Router::new().route("/chat/completions", post(handle)).with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeUpstream { base_url: format!("http://127.0.0.1:{port}"), state }
    }

    /// Every request body received, in order.
    pub fn requests(&self) -> Vec<Value> {
        self.state.seen.lock().unwrap().clone()
    }

    /// The Authorization header of each request (None when absent).
    pub fn auth_headers(&self) -> Vec<Option<String>> {
        self.state.headers.lock().unwrap().clone()
    }
}

async fn handle(
    State(st): State<Arc<St>>,
    headers: axum::http::HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let n = {
        let mut seen = st.seen.lock().unwrap();
        seen.push(body);
        seen.len()
    };
    st.headers
        .lock()
        .unwrap()
        .push(headers.get("authorization").and_then(|v| v.to_str().ok()).map(str::to_string));
    let Some(msg) = st.script.get(n - 1).or(st.script.last()) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": {"message": "empty script"}}))).into_response();
    };
    if let Some(code) = msg.get("_status").and_then(Value::as_u64) {
        return (
            StatusCode::from_u16(code as u16).unwrap(),
            Json(json!({"error": {"message": "scripted upstream failure"}})),
        )
            .into_response();
    }
    let mut msg = msg.clone();
    let finish = msg
        .as_object_mut()
        .and_then(|m| m.remove("_finish_reason"))
        .unwrap_or_else(|| json!("stop"));
    Json(json!({
        "id": format!("gen-{n}"),
        "model": "fake/model",
        "choices": [{"index": 0, "message": msg, "finish_reason": finish}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15, "cost": 0.001},
    }))
    .into_response()
}
