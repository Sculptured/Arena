//! Experiments over HTTP: create one (`POST /api/experiments`), list them, read one, stop one.
//! Suites made on their own show up as one-setup experiments (`legacy`).

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use parts::proto::ScenarioRequest;
use serde::Deserialize;

use super::{check_disk, detached, parse_body, settle_providers, validate_builtin_team};
use super::{ApiError, ApiResult};
use crate::{
    app::AppState,
    experiment::{self, Detail, Listed, DEFAULT_RUNS, MAX_SETUPS},
    model::{Runner, Team},
    store::{NewExperiment, NewSetup},
    suite::MAX_REPEATS,
};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/experiments", get(list).post(create))
        .route("/api/experiments/{id}", get(get_one))
        .route("/api/experiments/{id}/stop", post(stop))
}

fn default_runs() -> i32 {
    DEFAULT_RUNS
}

/// One agent setup: its name (`Setup A`, … when absent), team and runner (`chain` by default).
#[derive(Deserialize)]
struct SetupReq {
    #[serde(default)]
    name: Option<String>,
    team: Team,
    #[serde(default)]
    runner: Runner,
}

/// A new experiment: the task, the disruptions every run adds to it, the runs per setup, and
/// the setups.
#[derive(Deserialize)]
struct NewExperimentReq {
    #[serde(default)]
    name: Option<String>,
    task: String,
    #[serde(default)]
    scenario: Option<ScenarioRequest>,
    #[serde(default = "default_runs")]
    runs: i32,
    setups: Vec<SetupReq>,
}

/// 400 unless arenad can run the setup itself: a built-in runner and a team it can run. The
/// error names the setup.
fn validate_setup(state: &AppState, name: &str, setup: &SetupReq) -> Result<(), ApiError> {
    if setup.runner == Runner::External {
        return Err(ApiError::bad(format!(
            "{name}: an experiment's setups run on arenad's runners"
        )));
    }
    validate_builtin_team(state, &setup.team, setup.runner)
        .map_err(|ApiError(code, msg)| ApiError(code, format!("{name}: {msg}")))
}

/// The setups' names, after checking their number, the runs and each setup (400 otherwise).
fn validate(state: &AppState, req: &mut NewExperimentReq) -> Result<Vec<String>, ApiError> {
    if !(1..=MAX_SETUPS).contains(&req.setups.len()) {
        return Err(ApiError::bad(format!(
            "an experiment has 1 to {MAX_SETUPS} setups"
        )));
    }
    if !(1..=MAX_REPEATS).contains(&req.runs) {
        return Err(ApiError::bad(format!(
            "runs must be from 1 to {MAX_REPEATS}"
        )));
    }
    let asked: Vec<Option<&str>> = req.setups.iter().map(|s| s.name.as_deref()).collect();
    let names = experiment::setup_names(&asked).map_err(ApiError::bad)?;
    for (name, setup) in names.iter().zip(&mut req.setups) {
        settle_providers(&mut setup.team);
        validate_setup(state, name, setup)?;
    }
    Ok(names)
}

/// `POST /api/experiments`: stores the experiment (a suite per setup) and starts running it.
/// Answers its detail.
async fn create(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Detail> {
    let mut req: NewExperimentReq = parse_body(&body)?;
    let names = validate(&state, &mut req)?;
    super::suites::suite_tasks(&state, vec![req.task.clone()]).await?;
    let scenario = super::twin::scenario_of(&state, req.scenario.as_ref()).await?;
    check_disk(&state).await?;
    let setups: Vec<NewSetup> = (names.iter().zip(&req.setups))
        .map(|(name, s)| NewSetup {
            name,
            team: &s.team,
            runner: s.runner,
        })
        .collect();
    let new = NewExperiment {
        name: &experiment::name_of(req.name.as_deref(), &names),
        task: &req.task,
        runs: req.runs,
        benchmark: &state.benchmark,
        scenario: &scenario,
        setups: &setups,
    };
    let (stored, suites) = state.store.create_experiment(new).await?;
    experiment::spawn(&state, stored.id.clone(), suites);
    Ok(Json(experiment::detail(&state.store, &stored).await?))
}

/// `GET /api/experiments`: every experiment and suite made on its own, newest first.
async fn list(State(state): State<Arc<AppState>>) -> ApiResult<Vec<Listed>> {
    Ok(Json(experiment::list(&state.store).await?))
}

/// What an id names: an experiment, or a suite made on its own (a legacy one-setup experiment).
enum Found {
    Experiment(experiment::Experiment),
    Legacy(crate::suite::Suite),
}

async fn find(state: &AppState, id: &str) -> Result<Found, ApiError> {
    if let Some(e) = state.store.experiment(id).await? {
        return Ok(Found::Experiment(e));
    }
    match state.store.suite(id).await? {
        Some(s) if s.experiment_id.is_none() => Ok(Found::Legacy(s)),
        _ => Err(ApiError::not_found(format!(
            "there is no experiment \"{id}\""
        ))),
    }
}

async fn detail_of(state: &AppState, found: Found) -> ApiResult<Detail> {
    let d = match found {
        Found::Experiment(e) => experiment::detail(&state.store, &e).await?,
        Found::Legacy(s) => experiment::legacy_detail(&state.store, s).await?,
    };
    Ok(Json(d))
}

/// `GET /api/experiments/{id}`: the experiment with its setups' suite reports.
async fn get_one(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Detail> {
    let found = find(&state, &id).await?;
    detail_of(&state, found).await
}

/// `POST /api/experiments/{id}/stop`: no further run starts and the running one is stopped.
/// Answers the experiment's detail; 409 for a suite made on its own.
async fn stop(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Detail> {
    let Found::Experiment(_) = find(&state, &id).await? else {
        return Err(ApiError::conflict(
            "a suite made on its own is not an experiment arenad can stop",
        ));
    };
    detached(async move {
        experiment::stop(&state, &id).await?;
        let found = find(&state, &id).await?;
        detail_of(&state, found).await
    })
    .await
}
