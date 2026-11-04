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
    exec::{tool_schemas, BranchCtx, ToolExec},
    overhead::{self, ms_since, Cat},
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new().route("/mcp/{branch}/{agent}", post(handle))
}

fn rpc_error(id: Value, code: i64, msg: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}})
}

/// The JSON-RPC reply to one request; `None` for a notification (nothing to answer). The time
/// spent running a tool goes into `ran_ms`.
async fn reply_to(
    state: &AppState,
    ctx: &BranchCtx,
    agent: &str,
    req: &Value,
    ran_ms: &mut f64,
) -> Option<Value> {
    let id = req.get("id").cloned()?;
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, initialize_result(ctx)),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({"tools": tool_schemas(ctx, agent)})),
        "tools/call" => {
            let since = Instant::now();
            let result = call_tool(state, ctx, agent, &id, &params).await;
            *ran_ms = ms_since(since);
            ok(id, result)
        }
        _ => rpc_error(id, -32601, &format!("method not found: {method}")),
    })
}

async fn handle(
    State(state): State<Arc<AppState>>,
    Path((branch, agent)): Path<(String, String)>,
    body: String,
) -> Response {
    let since = Instant::now();
    let ctx: Option<Arc<BranchCtx>> = state.live.read().unwrap().get(&branch).cloned();
    let Some(ctx) = ctx.filter(|c| c.has_agent(&agent)) else {
        return (
            StatusCode::NOT_FOUND,
            Json(rpc_error(Value::Null, -32000, "unknown branch or agent")),
        )
            .into_response();
    };
    let req: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            return Json(rpc_error(Value::Null, -32700, &format!("parse error: {e}")))
                .into_response()
        }
    };
    let mut ran_ms = 0.0;
    let reply = reply_to(&state, &ctx, &agent, &req, &mut ran_ms).await;
    // The endpoint's own handling: everything but running the tool.
    overhead::add(&ctx.branch_id, Cat::Mcp, ms_since(since) - ran_ms);
    match reply {
        Some(reply) => Json(reply).into_response(),
        // Notification (or stray response): nothing to answer.
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn initialize_result(ctx: &BranchCtx) -> Value {
    json!({
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": format!("arena-{}", ctx.world.bench()), "version": "0.1.0"},
    })
}

/// Runs a `tools/call`; a tool error is a result with `isError`, not an RPC error.
async fn call_tool(
    state: &AppState,
    ctx: &BranchCtx,
    agent: &str,
    id: &Value,
    params: &Value,
) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let exec = ToolExec::new(state.store.clone());
    // A missing name is just an unknown tool: it is recorded like any bad call.
    let res = exec
        .run_outcome(ctx, agent, name, args, Some(id_string(id)))
        .await;
    tool_result(res)
}

/// A tool's outcome as an MCP `tools/call` result.
fn tool_result(res: Result<Value, String>) -> Value {
    match res {
        Ok(v) => {
            let mut r =
                json!({"content": [{"type": "text", "text": v.to_string()}], "isError": false});
            if v.is_object() {
                r["structuredContent"] = v;
            }
            r
        }
        Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
    }
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn id_string(id: &Value) -> String {
    id.as_str()
        .map(str::to_string)
        .unwrap_or_else(|| id.to_string())
}
