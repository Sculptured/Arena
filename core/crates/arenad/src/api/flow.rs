//! Pausing, resuming and stopping a live branch (`POST /api/branches/{id}/pause`, `/resume`,
//! `/stop`), whatever drives its agents.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};

use super::{ApiError, ApiResult};
use crate::{app::AppState, exec::BranchCtx, runner};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/branches/{id}/pause", post(pause))
        .route("/api/branches/{id}/resume", post(resume))
        .route("/api/branches/{id}/stop", post(stop))
}

/// The live branch `id`: 409 when it has ended (or never ran here), 404 when there is none.
async fn live(state: &AppState, id: &str) -> Result<Arc<BranchCtx>, ApiError> {
    if let Some(ctx) = state.live.read().unwrap().get(id).cloned() {
        return Ok(ctx);
    }
    match state.store.find_branch(id).await? {
        Some(_) => Err(ApiError::conflict("the run is not running")),
        None => Err(ApiError::not_found(format!("there is no branch \"{id}\""))),
    }
}

/// Whether the branch is paused now, and whether the request changed that.
fn paused(ctx: &BranchCtx, changed: bool) -> ApiResult<Value> {
    Ok(Json(
        json!({"paused": ctx.flow.is_paused(), "changed": changed}),
    ))
}

/// `POST /api/branches/{id}/pause`: new tool calls and model calls wait, the world clock stops.
async fn pause(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let ctx = live(&state, &id).await?;
    paused(&ctx, ctx.pause(&state.store).await?)
}

/// `POST /api/branches/{id}/resume`: the calls that wait go on, the world clock runs again.
async fn resume(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let ctx = live(&state, &id).await?;
    paused(&ctx, ctx.resume(&state.store).await?)
}

/// `POST /api/branches/{id}/stop`: the run ends `stopped`, unscored. arenad's own runner ends
/// the branch on its task; the stream says when it has.
async fn stop(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let ctx = live(&state, &id).await?;
    let stopped = super::detached(async move { Ok(runner::stop(&state, &ctx).await) }).await?;
    Ok(Json(json!({"stopping": stopped})))
}

/// True when the live branch `id` is paused (a branch that isn't live never is).
pub(super) fn is_paused(state: &AppState, id: &str) -> bool {
    let live = state.live.read().unwrap();
    live.get(id).is_some_and(|c| c.flow.is_paused())
}
