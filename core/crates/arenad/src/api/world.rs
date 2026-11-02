//! The world, its disk usage and its cleanup, for the control page.

use std::sync::Arc;

use axum::{extract::State, Json};
use parts::proto::{Describe, ExactKind, WorldPart};
use serde::Serialize;
use serde_json::{json, Value};

use super::{detached, ApiError, ApiResult};
use crate::{app::AppState, gc, saves};

#[derive(Debug, Serialize)]
struct Restore {
    exact: ExactKind,
    approximate: bool,
}

/// One part of the benchmark's world, with what its integration said about itself. A part whose
/// integration did not answer has only `name`, `integration`, `answered: false` and `error`.
#[derive(Debug, Serialize)]
struct PartInfo {
    name: String,
    integration: String,
    answered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    restore: Option<Restore>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uses_clock: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<String>>,
}

impl PartInfo {
    fn new(part: &WorldPart, described: Result<Describe, String>) -> PartInfo {
        let mut info = PartInfo {
            name: part.part.clone(),
            integration: part.integration.clone(),
            answered: false,
            error: None,
            version: None,
            restore: None,
            uses_clock: None,
            tools: None,
        };
        match described {
            Ok(d) => {
                info.answered = true;
                info.version = Some(d.version);
                info.restore = Some(Restore {
                    exact: d.restore.exact,
                    approximate: d.restore.approximate,
                });
                info.uses_clock = Some(d.uses_clock);
                info.tools = Some(d.tools.into_iter().map(|t| t.name).collect());
            }
            Err(e) => info.error = Some(e),
        }
        info
    }
}

/// `GET /api/world`: the benchmark's parts, each with its integration's `describe`.
pub async fn world(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    let mut parts = Vec::new();
    for p in &bench.world {
        let described = state
            .hub
            .describe(&p.integration)
            .await
            .map_err(|e| format!("{e:#}"));
        parts.push(PartInfo::new(p, described));
    }
    Ok(Json(json!({ "parts": parts })))
}

/// `GET /api/usage`: the bytes each integration of the world keeps (its own plus its save
/// folders), their total and the disk limit.
pub async fn usage(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let mut parts = Vec::new();
    let mut total = 0;
    for i in saves::integrations(&state.hub, &state.benchmark).await? {
        let bytes = saves::usage(&state.hub, &i).await? + state.hub.package_saves_bytes(&i);
        total += bytes;
        parts.push(json!({"name": i, "bytes": bytes}));
    }
    Ok(Json(json!({
        "parts": parts,
        "total_bytes": total,
        "limit_bytes": state.config.max_db_bytes,
    })))
}

/// `POST /api/gc`: the startup cleanup (it keeps every save a recorded run names, and the shared
/// base and gold saves). 409 while a branch is running, as its instances must not be dropped.
pub async fn gc(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    if !state.live.read().unwrap().is_empty() {
        return Err(ApiError::conflict(
            "agents are running; clean up when they have stopped",
        ));
    }
    detached(async move {
        let report = gc::run_gc(&state.store, &state.hub, &state.benchmark).await?;
        Ok(Json(json!({
            "removed": report.removed,
            "freed_bytes": report.result.bytes,
        })))
    })
    .await
}
