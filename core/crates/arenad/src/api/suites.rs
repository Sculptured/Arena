//! Test suites over HTTP: create one (`POST /api/suites`), list them, read one's report, claim
//! an external suite's next run, and compare two suites.

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{
    check_disk, detached, not_cleaning, parse_body, settle_providers, validate_agents,
    validate_builtin_team, ApiError, ApiResult,
};
use crate::{
    app::AppState,
    model::*,
    scenario::Scenario,
    store::NewSuite,
    suite::{self, Comparison, Listed, Suite, SuiteReport, MAX_REPEATS},
};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/suites", get(list).post(create))
        .route("/api/suites/compare", get(compare))
        .route("/api/suites/{id}", get(get_one))
        .route("/api/suites/{id}/claim", post(claim))
}

fn one() -> i32 {
    1
}

/// A new suite: the team (a deployment), the benchmark's tasks it runs (all when empty), how many
/// times each, and the runner (`chain` by default; `external`: an external system claims each run).
#[derive(Deserialize)]
struct NewSuiteReq {
    #[serde(default)]
    name: Option<String>,
    team: Team,
    #[serde(default)]
    tasks: Vec<String>,
    #[serde(default = "one")]
    repeats: i32,
    #[serde(default)]
    runner: Runner,
    /// The disruptions every run adds to its task (`{disruptions: [{id, params}]}`).
    #[serde(default)]
    scenario: Option<parts::proto::ScenarioRequest>,
    /// Several scenarios instead: each task × repeat runs once in each (`scenario` must be absent).
    #[serde(default)]
    scenarios: Vec<parts::proto::ScenarioRequest>,
}

/// The most scenarios a suite may vary its runs over.
const MAX_VARIANTS: usize = 20;

/// The suite's one scenario and its variants, each expanded; 400 for both at once, too many, or
/// one that can't be expanded.
async fn suite_scenarios(
    state: &AppState,
    req: &NewSuiteReq,
) -> Result<(Scenario, Vec<Scenario>), ApiError> {
    let one = req.scenario.as_ref().filter(|s| !s.disruptions.is_empty());
    if one.is_some() && !req.scenarios.is_empty() {
        return Err(ApiError::bad(
            "give a suite either scenario or scenarios, not both",
        ));
    }
    if req.scenarios.len() > MAX_VARIANTS {
        return Err(ApiError::bad(format!("at most {MAX_VARIANTS} scenarios")));
    }
    let mut variants = vec![];
    for asked in &req.scenarios {
        variants.push(super::twin::scenario_of(state, Some(asked)).await?);
    }
    Ok((super::twin::scenario_of(state, one).await?, variants))
}

/// The tasks a suite runs: the ones asked for (each once, each the benchmark's), or all.
async fn suite_tasks(state: &AppState, asked: Vec<String>) -> Result<Vec<String>, ApiError> {
    let known: Vec<String> = state
        .bench()
        .await?
        .tasks
        .into_iter()
        .map(|t| t.id)
        .collect();
    if asked.is_empty() {
        return Ok(known);
    }
    for (i, t) in asked.iter().enumerate() {
        if !known.contains(t) {
            return Err(ApiError::bad(format!("the benchmark has no task {t:?}")));
        }
        if asked[..i].contains(t) {
            return Err(ApiError::bad(format!("task {t:?} is listed twice")));
        }
    }
    Ok(asked)
}

/// 400 unless the request's repeats and team suit its runner.
fn validate_suite(state: &AppState, req: &NewSuiteReq) -> Result<(), ApiError> {
    if !(1..=MAX_REPEATS).contains(&req.repeats) {
        return Err(ApiError::bad(format!(
            "repeats must be from 1 to {MAX_REPEATS}"
        )));
    }
    match req.runner {
        Runner::External => validate_agents(state, &req.team.agents),
        runner => validate_builtin_team(state, &req.team, runner),
    }
}

/// `POST /api/suites`: stores the suite and, for a built-in runner, starts running it. Answers
/// its report.
async fn create(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<SuiteReport> {
    let mut req: NewSuiteReq = parse_body(&body)?;
    settle_providers(&mut req.team);
    validate_suite(&state, &req)?;
    let (scenario, variants) = suite_scenarios(&state, &req).await?;
    let tasks = suite_tasks(&state, std::mem::take(&mut req.tasks)).await?;
    if req.runner != Runner::External {
        check_disk(&state).await?;
    }
    let name = req
        .name
        .as_deref()
        .and_then(trimmed)
        .unwrap_or(req.team.name.clone());
    let new = NewSuite {
        name: &name,
        team: &req.team,
        tasks: &tasks,
        repeats: req.repeats,
        runner: req.runner,
        benchmark: &state.benchmark,
        scenario: &scenario,
        variants: &variants,
    };
    let stored = state.store.create_suite(new).await?;
    if stored.runner != Runner::External {
        suite::spawn_builtin(&state, stored.clone());
    }
    Ok(Json(suite::load(&state.store, stored).await?))
}

/// `GET /api/suites`: every suite, newest first, with its status and how many runs ended.
async fn list(State(state): State<Arc<AppState>>) -> ApiResult<Vec<Listed>> {
    let mut out = vec![];
    for s in state.store.suites().await? {
        out.push(suite::listed(&state.store, s).await?);
    }
    Ok(Json(out))
}

async fn find(state: &AppState, id: &str) -> Result<Suite, ApiError> {
    state
        .store
        .suite(id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("there is no suite \"{id}\"")))
}

/// `GET /api/suites/{id}`: the suite's report.
async fn get_one(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<SuiteReport> {
    let s = find(&state, &id).await?;
    Ok(Json(suite::load(&state.store, s).await?))
}

/// `POST /api/suites/{id}/claim`: an external suite's next run, started: `{done: false, task_id,
/// repeat, run_id, branch_id, agents}`, which the system drives through the external-run API;
/// `{done: true}` when every run is claimed.
async fn claim(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let s = find(&state, &id).await?;
    if s.runner != Runner::External {
        return Err(ApiError::conflict("arenad runs this suite's runs itself"));
    }
    let cleanup = not_cleaning(&state)?;
    check_disk(&state).await?;
    detached(async move {
        let _cleanup = cleanup;
        let Some(claimed) = suite::claim_external(&state, &s).await? else {
            return Ok(Json(json!({"done": true})));
        };
        let mut v = serde_json::to_value(claimed).map_err(anyhow::Error::from)?;
        v["done"] = json!(false);
        Ok(Json(v))
    })
    .await
}

/// `GET /api/suites/compare?a=&b=`: suite `b` against suite `a`.
async fn compare(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Comparison> {
    let (Some(a), Some(b)) = (q.get("a"), q.get("b")) else {
        return Err(ApiError::bad("a and b are required"));
    };
    let a = suite::load(&state.store, find(&state, a).await?).await?;
    let b = suite::load(&state.store, find(&state, b).await?).await?;
    Ok(Json(suite::compare(&a, &b)))
}
