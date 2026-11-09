//! Two finished branches compared: their summaries, where they diverged, and the rows their final
//! worlds differ in (`GET /api/compare?a=&b=`).

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Query, State},
    Json,
};
use serde_json::{json, Value};

use super::{branch_or_404, pair, ApiError, ApiResult};
use crate::{
    app::AppState,
    compare::{self, FinalState},
    model::*,
    recorded::final_saves,
    saves::{part_code, world_of},
};

const NO_FINAL_STATE: &str = "this branch has no final state yet";

/// The branch, which must exist (404) and have finished (409).
pub(super) async fn finished_branch(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    let b = branch_or_404(state, id).await?;
    if b.status == BranchStatus::Running {
        return Err(ApiError::conflict(format!(
            "branch \"{id}\" is still running"
        )));
    }
    Ok(b)
}

/// One side of a comparison: the branch, its events and its final state (409 without one).
pub(super) struct Side {
    pub branch: Branch,
    pub events: Vec<Event>,
    pub end: FinalState,
}

async fn side(state: &AppState, branch: Branch) -> Result<Side, ApiError> {
    let events = state.store.events(&branch.id).await?;
    let saves = final_saves(&branch.id, &events).ok_or(ApiError::conflict(NO_FINAL_STATE))?;
    let world = world_of(&state.hub, &events).await?;
    Ok(Side {
        branch,
        events,
        end: FinalState { world, saves },
    })
}

/// Both sides: existence first (404), then finished (409), then a final state (409).
pub(super) async fn sides(state: &AppState, ids: [&String; 2]) -> Result<[Side; 2], ApiError> {
    for id in ids {
        branch_or_404(state, id).await?;
    }
    let (a, b) = (
        finished_branch(state, ids[0]).await?,
        finished_branch(state, ids[1]).await?,
    );
    Ok([side(state, a).await?, side(state, b).await?])
}

pub(super) async fn compare_branches(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let (a, b) = pair(&q)?;
    let [a, b] = sides(&state, [&a, &b]).await?;
    let skipped = compare::not_compared(&state.hub, &a.end, &b.end).await?;
    let rows = compared_rows(&state, &a.end, &b.end, &skipped).await?;
    Ok(Json(json!({
        "a": compare::summary(&a.branch, &a.events),
        "b": compare::summary(&b.branch, &b.events),
        "first_divergent_seq": compare::first_divergent_seq(&a.events, &b.events),
        "differing_rows": rows,
        "not_compared": skipped,
    })))
}

/// The differing rows of two final states, but the parts in `skip`: 409 when a branch lacks a
/// final save of a compared part, or a save is gone.
pub(super) async fn compared_rows(
    state: &AppState,
    a: &FinalState,
    b: &FinalState,
    skip: &[String],
) -> Result<Vec<Value>, ApiError> {
    if let Some(part) = compare::missing_part(a, b, skip) {
        return Err(ApiError::conflict(format!(
            "a branch has no final state of the part {part}"
        )));
    }
    match compare::differing_rows(&state.hub, a, b, skip).await {
        // A save that is gone: a branch recorded before integrations whose database is missing.
        Err(e) if part_code(&e) == Some(-32002) => Err(ApiError::conflict(NO_FINAL_STATE)),
        r => Ok(r?),
    }
}
