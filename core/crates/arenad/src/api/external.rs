//! The routes an external system drives a run with: start it (`POST /api/runs` with `runner:
//! "external"`), register agents and get their endpoints, report an agent done, and finish it.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::OwnedRwLockReadGuard;

use super::{
    check_disk, detached, parse_body, validate_agent_id, validate_agents, ApiError, ApiResult,
};
use crate::{
    app::AppState,
    exec::{tool_schemas, BranchCtx},
    external::{self, endpoints, AgentDone, Join, UsageReport},
    model::*,
    runner,
};

pub(super) fn router() -> Router<Arc<AppState>> {
    let agent = "/api/branches/{id}/agents/{agent}";
    Router::new()
        .route("/api/branches/{id}/agents", post(register))
        .route(&format!("{agent}/end"), post(end_agent))
        .route(&format!("{agent}/usage"), post(report_usage))
        .route("/api/branches/{id}/finish", post(finish))
}

/// Starts a run whose agents an external system drives: its world started, its clock ticking,
/// the team's agents (there may be none yet) registered. Answers `{run_id, branch_id, agents}`
/// with each agent's endpoints.
pub(super) async fn create_run(
    state: Arc<AppState>,
    cleanup: OwnedRwLockReadGuard<()>,
    task_id: String,
    team: Team,
    scenario: crate::scenario::Scenario,
) -> ApiResult<Value> {
    validate_agents(&state, &team.agents)?;
    check_disk(&state).await?;
    detached(async move {
        let _cleanup = cleanup;
        let started = external::start(&state, &task_id, team, &scenario).await?;
        Ok(Json(
            serde_json::to_value(started).map_err(anyhow::Error::from)?,
        ))
    })
    .await
}

/// The live branch `id` an external system drives: 404 when it isn't live, 409 when arenad runs
/// its agents.
fn external_branch(state: &AppState, id: &str) -> Result<Arc<BranchCtx>, ApiError> {
    let ctx = state.live.read().unwrap().get(id).cloned();
    let ctx = ctx.ok_or_else(|| ApiError::not_found(format!("no live branch \"{id}\"")))?;
    if ctx.rules.runner != Runner::External {
        return Err(ApiError::conflict(
            "arenad runs this branch's agents itself",
        ));
    }
    Ok(ctx)
}

/// `POST /api/branches/{id}/agents`: registers an agent (or starts a known one again) and
/// answers its endpoints and the tools it gets.
async fn register(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Value> {
    let join: Join = parse_body(&body)?;
    validate_agent_id(&join.id)?;
    let ctx = external_branch(&state, &id)?;
    if ctx.agent(&join.id).is_none() {
        validate_agents(&state, &[join.spec()])?;
    }
    let spec = external::register(&state.store, &ctx, &join).await?;
    let mut v = serde_json::to_value(endpoints(&state.config.self_base, &id, &spec.id))
        .map_err(anyhow::Error::from)?;
    v["tools"] = json!(tool_schemas(&ctx, &spec.id));
    v["spec"] = serde_json::to_value(&spec).map_err(anyhow::Error::from)?;
    Ok(Json(v))
}

/// `POST /api/branches/{id}/agents/{agent}/end`: records that the agent is done, with the tokens
/// and cost the system reports for it.
async fn end_agent(
    State(state): State<Arc<AppState>>,
    Path((id, agent)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Value> {
    let done: AgentDone = if body.is_empty() {
        AgentDone {
            ok: true,
            ..AgentDone::default()
        }
    } else {
        parse_body(&body)?
    };
    let ctx = external_branch(&state, &id)?;
    if !ctx.has_agent(&agent) {
        return Err(ApiError::not_found(format!(
            "no agent \"{agent}\" on this branch"
        )));
    }
    let end = serde_json::to_value(done.payload()).map_err(anyhow::Error::from)?;
    let ev = ctx.append(&state.store, Some(&agent), EventKind::AgentEnd, end);
    Ok(Json(json!({"seq": ev.await?.seq})))
}

/// `POST /api/branches/{id}/agents/{agent}/usage`: records one model call (or compaction) the
/// agent's client reported, as a `ModelCall` event.
async fn report_usage(
    State(state): State<Arc<AppState>>,
    Path((id, agent)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<Value> {
    let report: UsageReport = parse_body(&body)?;
    let ctx = external_branch(&state, &id)?;
    if !ctx.has_agent(&agent) {
        return Err(ApiError::not_found(format!(
            "no agent \"{agent}\" on this branch"
        )));
    }
    let ev = ctx.append(
        &state.store,
        Some(&agent),
        EventKind::ModelCall,
        report.payload(),
    );
    Ok(Json(json!({"seq": ev.await?.seq})))
}

/// `POST /api/branches/{id}/finish`: the run is over. arenad freezes and scores the world, stops
/// it and answers how the branch ended.
async fn finish(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let ctx = external_branch(&state, &id)?;
    detached(async move {
        let ended = runner::finish(&state, &ctx).await;
        let (status, error) =
            ended.ok_or_else(|| ApiError::conflict("the branch is ending already"))?;
        Ok(Json(json!({"status": status, "error": error})))
    })
    .await
}
