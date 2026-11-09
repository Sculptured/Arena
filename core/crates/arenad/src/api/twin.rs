//! The scenario builder's and the digital twin's routes: the benchmark's scenario options
//! (`GET /api/benchmark`), a scenario asked for checked and expanded, and a branch's recorded
//! views (`GET /api/branches/{id}/views`).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use parts::proto::ScenarioRequest;
use serde_json::{json, Value};

use super::{ApiError, ApiResult};
use crate::{
    app::AppState,
    model::{Event, EventKind},
    scenario::{self, Scenario},
};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/benchmark", get(benchmark))
        .route("/api/branches/{id}/views", get(branch_views))
}

/// `GET /api/benchmark`: `{name, version, tasks: [{id, reason}], scenario_options}` (null when
/// the benchmark offers no disruptions; `view`: whether it draws the world's view).
async fn benchmark(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    let tasks: Vec<Value> = (bench.tasks.iter())
        .map(|t| json!({"id": t.id, "reason": t.reason}))
        .collect();
    Ok(Json(json!({
        "name": bench.name,
        "version": bench.version,
        "tasks": tasks,
        "scenario_options": bench.scenario_options,
        "view": bench.view.is_some(),
    })))
}

/// The scenario `asked` expanded with the benchmark's options (empty when none was asked); 400
/// with why when it can't be.
pub(super) async fn scenario_of(
    state: &AppState,
    asked: Option<&ScenarioRequest>,
) -> Result<Scenario, ApiError> {
    let Some(asked) = asked.filter(|a| !a.disruptions.is_empty()) else {
        return Ok(Scenario::default());
    };
    let bench = state.bench().await?;
    scenario::expand(bench.scenario_options.as_ref(), asked).map_err(ApiError::bad)
}

/// `[{seq, event_seq, view}]` of a branch's `world_view` events, in order: `seq` is the step the
/// view follows (`seq_of`), `event_seq` the view's own event.
pub fn views(events: &[Event]) -> Vec<Value> {
    let of = |e: &Event| {
        let seq = e.payload["seq_of"].as_i64().unwrap_or(e.seq);
        json!({"seq": seq, "event_seq": e.seq, "view": e.payload["view"]})
    };
    (events.iter())
        .filter(|e| e.kind == EventKind::WorldView)
        .map(of)
        .collect()
}

/// `GET /api/branches/{id}/views`: the branch's views, those it inherited before its fork point
/// included (a fork's record starts with its parent's events before that point).
async fn branch_views(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<Value>> {
    super::branch_or_404(&state, &id).await?;
    let events = state.store.events(&id).await?;
    Ok(Json(views(&events)))
}
