use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use world::tools;

use crate::{
    app::AppState,
    exec::{handoff_schema, BranchCtx, ToolExec},
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/mcp/{branch}/{agent}", post(handle))
}

fn rpc_error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    body: String,
) -> Response {
    let ctx: Option<Arc<BranchCtx>> = state.live.read().unwrap().get(&branch).cloned();
    let Some(ctx) = ctx.filter(|c| c.has_agent(&agent)) else {
        return (StatusCode::NOT_FOUND, Json(rpc_error(Value::Null, -32000, "unknown branch or agent")))
            .into_response();
    };
    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return Json(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))).into_response(),
    };
    let Some(id) = req.get("id").cloned() else {
        // Notification (or stray response): nothing to answer.
        return StatusCode::ACCEPTED.into_response();
    };
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let reply = match method {
        "initialize" => ok(id, json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "arena-retail", "version": "0.1.0"},
        })),
        "ping" => ok(id, json!({})),
        "tools/list" => {
            let mut list = tools::tool_schemas();
            if !ctx.is_last(&agent) {
                list.push(handoff_schema());
            }
            ok(id, json!({"tools": list}))
        }
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let exec = ToolExec::new(state.store.clone(), state.worlds.clone());
            // A missing name is just an unknown tool: it is recorded like any bad call.
            let res = exec.run_outcome(&ctx, &agent, &name, args, Some(id_string(&id))).await;
            ok(id, match res {
                Ok(v) => {
                    let mut r = json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false});
                    if v.is_object() {
                        r["structuredContent"] = v;
                    }
                    r
                }
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        _ => rpc_error(id, -32601, &format!("method not found: {method}")),
    };
    Json(reply).into_response()
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn id_string(id: &Value) -> String {
    id.as_str().map(str::to_string).unwrap_or_else(|| id.to_string())
}
