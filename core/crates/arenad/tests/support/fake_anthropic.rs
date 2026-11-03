//! A scripted Claude Messages API for tests. `POST /messages` serves the script's content
//! blocks in order (repeating the last entry when exhausted) as messages with fixed usage; an
//! entry is `{"content": [...], "stop_reason": "..."}`, with `stop_details` passed on when it
//! has them. Every request body is recorded with its
//! `x-api-key`, `anthropic-version` and `authorization` headers. `GET /models` lists
//! `claude-b` and `claude-a`, and records the same headers.

use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    http::HeaderMap,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};

/// The headers a request came with: x-api-key, anthropic-version, authorization.
pub type Headers = [Option<String>; 3];

pub struct FakeAnthropic {
    pub base_url: String,
    state: Arc<St>,
}

struct St {
    script: Vec<Value>,
    seen: Mutex<Vec<(Value, Headers)>>,
    listed: Mutex<Vec<Headers>>,
}

impl FakeAnthropic {
    pub async fn start(script: Vec<Value>) -> FakeAnthropic {
        let state = Arc::new(St {
            script,
            seen: Mutex::new(vec![]),
            listed: Mutex::new(vec![]),
        });
        let app = Router::new()
            .route("/messages", post(message))
            .route("/models", get(models))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeAnthropic {
            base_url: format!("http://127.0.0.1:{port}"),
            state,
        }
    }

    /// Every `POST /messages` body, with its headers, in order.
    pub fn requests(&self) -> Vec<(Value, Headers)> {
        self.state.seen.lock().unwrap().clone()
    }

    /// The headers of every `GET /models`.
    pub fn list_headers(&self) -> Vec<Headers> {
        self.state.listed.lock().unwrap().clone()
    }
}

fn headers_of(h: &HeaderMap) -> Headers {
    ["x-api-key", "anthropic-version", "authorization"]
        .map(|k| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string))
}

async fn models(State(st): State<Arc<St>>, headers: HeaderMap) -> Json<Value> {
    st.listed.lock().unwrap().push(headers_of(&headers));
    Json(
        json!({"data": [{"id": "claude-b", "type": "model"}, {"id": "claude-a", "type": "model"}],
                "has_more": false}),
    )
}

async fn message(
    State(st): State<Arc<St>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Json<Value> {
    let n = {
        let mut seen = st.seen.lock().unwrap();
        seen.push((body.clone(), headers_of(&headers)));
        seen.len()
    };
    let entry = st.script.get(n - 1).or(st.script.last()).unwrap();
    let mut message = json!({
        "id": format!("msg_{n}"),
        "type": "message",
        "role": "assistant",
        "model": body["model"],
        "content": entry["content"],
        "stop_reason": entry["stop_reason"],
        "usage": {"input_tokens": 10, "output_tokens": 5, "cache_read_input_tokens": 2},
    });
    if let Some(details) = entry.get("stop_details") {
        message["stop_details"] = details.clone();
    }
    Json(message)
}
