//! The world, its disk usage and its cleanup, for the control page.

use std::{collections::BTreeMap, future::Future, sync::Arc, time::Duration};

use anyhow::Result;
use axum::{extract::State, Json};
use futures_util::future::join_all;
use parts::proto::{Describe, ExactKind, WorldPart};
use serde::Serialize;
use serde_json::{json, Value};

use super::{detached, ApiError, ApiResult, CLEANING_UP};
use crate::{app::AppState, gc, saves};

/// How long one integration gets to answer `describe` or `usage` before the page shows it as not
/// answering. A late answer is dropped by the client, so the integration is not harmed.
const DESCRIBE_TIMEOUT: Duration = Duration::from_secs(5);

/// `f`'s result, or the plain error text: its own, or that it did not answer in time.
async fn bounded<T>(f: impl Future<Output = Result<T>>) -> Result<T, String> {
    match tokio::time::timeout(DESCRIBE_TIMEOUT, f).await {
        Ok(r) => r.map_err(|e| format!("{e:#}")),
        Err(_) => Err(format!(
            "did not answer within {} s",
            DESCRIBE_TIMEOUT.as_secs()
        )),
    }
}

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

/// `GET /api/world`: the benchmark's parts, each with its integration's `describe`; the parts
/// are asked at the same time, each within `DESCRIBE_TIMEOUT`.
pub async fn world(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    let asked = bench
        .world
        .iter()
        .map(|p| async { PartInfo::new(p, bounded(state.hub.describe(&p.integration)).await) });
    let parts = join_all(asked).await;
    Ok(Json(json!({ "parts": parts })))
}

/// What one integration keeps: its own bytes plus its save folders; or why it can't say.
async fn integration_bytes(state: &AppState, integration: &str) -> Result<u64, String> {
    let own = bounded(saves::usage(&state.hub, integration)).await?;
    Ok(own + state.hub.package_saves_bytes(integration))
}

/// `GET /api/usage`: per integration of the world, the parts that use it and the bytes it keeps
/// (its own plus its save folders; `error` instead when it didn't answer), then the total and
/// the disk limit. Bytes belong to integrations, so a shared integration is counted once.
pub async fn usage(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    let mut users: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for p in &bench.world {
        users.entry(&p.integration).or_default().push(&p.part);
    }
    let st: &AppState = &state;
    let asked = users
        .iter()
        .map(|(i, parts)| async move { (i, parts, integration_bytes(st, i).await) });
    let mut total = 0;
    let mut integrations = Vec::new();
    for (integration, parts, bytes) in join_all(asked).await {
        let entry = match bytes {
            Ok(b) => {
                total += b;
                json!({"integration": integration, "parts": parts, "bytes": b})
            }
            Err(e) => json!({"integration": integration, "parts": parts, "bytes": 0, "error": e}),
        };
        integrations.push(entry);
    }
    Ok(Json(json!({
        "integrations": integrations,
        "total_bytes": total,
        "limit_bytes": state.config.max_db_bytes,
    })))
}

/// `POST /api/gc`: the startup cleanup (it keeps every save a recorded run names, and the shared
/// base and gold saves). Takes the cleanup lock, so no run or fork starts meanwhile; 409 while
/// one is starting or a branch is running, as gc drops instances. An integration whose gc fails
/// is listed in `errors`.
pub async fn gc(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let lock = state.cleanup.clone();
    let guard = lock
        .try_write_owned()
        .map_err(|_| ApiError::conflict(CLEANING_UP))?;
    if !state.live.read().unwrap().is_empty() {
        return Err(ApiError::conflict(
            "agents are running; clean up when they have stopped",
        ));
    }
    detached(async move {
        let _guard = guard;
        let report = gc::run_gc(&state.store, &state.hub, &state.benchmark).await?;
        Ok(Json(json!({
            "removed": report.removed,
            "freed_bytes": report.result.bytes,
            "errors": report.errors,
        })))
    })
    .await
}
