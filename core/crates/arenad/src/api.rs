//! The HTTP API the UI talks to. All JSON except the SSE stream.

use std::{collections::HashMap, sync::Arc};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use parts::proto::Mode;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::OwnedRwLockReadGuard;

use crate::{
    app::AppState,
    fork::{self, ForkError},
    gc, metrics,
    model::*,
    prices, prompts,
    providers::{self, ProviderStatus},
    runner,
};

mod compare;
mod experiments;
mod external;
mod flow;
mod names;
mod recording;
mod review;
mod rows;
mod sandbox;
mod stream;
mod suites;
mod twin;
mod ui;
mod world;

pub use ui::with_ui;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/providers", get(list_providers))
        .route("/api/tasks", get(tasks))
        .route("/api/team/default", get(default_team))
        .route("/api/teams", get(suggested_teams))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/{id}", axum::routing::delete(delete_run))
        .route("/api/runs/{id}/name", post(names::set))
        .route("/api/branches/{id}", get(get_branch))
        .route("/api/branches/{id}/stream", get(stream::stream))
        .route("/api/branches/{id}/rows", get(rows::rows))
        .route("/api/branches/{id}/fork", post(fork_branch))
        .route("/api/branches/{id}/restore-plan", get(get_restore_plan))
        .route("/api/compare", get(compare::compare_branches))
        .route("/api/world", get(world::world))
        .route("/api/usage", get(world::usage))
        .route("/api/gc", post(world::gc))
        .merge(external::router())
        .merge(flow::router())
        .merge(suites::router())
        .merge(experiments::router())
        .merge(review::router())
        .merge(sandbox::router())
        .merge(twin::router())
        .merge(recording::router())
        .route("/api/{*rest}", get(api_not_found).post(api_not_found))
}

// ---------------------------------------------------------------- errors

#[derive(Debug)]
pub struct ApiError(StatusCode, String);

impl ApiError {
    fn bad(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::BAD_REQUEST, msg.into())
    }
    fn not_found(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::NOT_FOUND, msg.into())
    }
    fn insufficient_storage(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::INSUFFICIENT_STORAGE, msg.into())
    }
    fn conflict(msg: impl Into<String>) -> ApiError {
        ApiError(StatusCode::CONFLICT, msg.into())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> ApiError {
        let status = match e.downcast_ref::<runner::UnknownTask>() {
            Some(_) => StatusCode::NOT_FOUND,
            None => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, format!("{e:#}"))
    }
}

impl From<ForkError> for ApiError {
    fn from(e: ForkError) -> ApiError {
        let status = match &e {
            ForkError::UnknownBranch(_) => StatusCode::NOT_FOUND,
            ForkError::BranchRunning
            | ForkError::Refused(_)
            | ForkError::ReplayDiverged { .. }
            | ForkError::FingerprintMismatch { .. } => StatusCode::CONFLICT,
            ForkError::UnknownSeq
            | ForkError::TeamChange(_)
            | ForkError::UnknownAgent(_)
            | ForkError::MissingHandoff(_)
            | ForkError::NoRestorePoint { .. } => StatusCode::BAD_REQUEST,
            ForkError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        ApiError(status, e.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

async fn api_not_found() -> ApiError {
    ApiError::not_found("no such route")
}

/// 404 for the branch `id` that isn't there.
fn no_branch(id: &str) -> ApiError {
    ApiError::not_found(format!("there is no branch \"{id}\""))
}

/// The branch `id`; 404 when there is none.
async fn branch_or_404(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    state
        .store
        .find_branch(id)
        .await?
        .ok_or_else(|| no_branch(id))
}

/// The query's `a` and `b` (a pair to compare); 400 without both.
fn pair(q: &HashMap<String, String>) -> Result<(String, String), ApiError> {
    match (q.get("a"), q.get("b")) {
        (Some(a), Some(b)) => Ok((a.clone(), b.clone())),
        _ => Err(ApiError::bad("a and b are required")),
    }
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|e| ApiError::bad(format!("bad request body: {e}")))
}

/// The reason a run or fork is refused while saves are being cleaned up.
const CLEANING_UP: &str = "Saves are being cleaned up; try again in a moment";

/// Held while a run or fork starts; 409 while a cleanup holds the lock.
fn not_cleaning(state: &AppState) -> Result<OwnedRwLockReadGuard<()>, ApiError> {
    state
        .cleanup
        .clone()
        .try_read_owned()
        .map_err(|_| ApiError::conflict(CLEANING_UP))
}

/// 400 unless an open agent can run on `provider` with `model`.
fn open_check(
    state: &AppState,
    provider: Option<&str>,
    model: Option<&str>,
) -> Result<(), ApiError> {
    match state.config.providers.refusal(provider, model) {
        None => Ok(()),
        Some(why) => Err(ApiError::bad(why)),
    }
}

/// 400 when the team the fork would run (`changes` applied to `parent`) puts a changed agent in
/// open mode on a provider that can't run it.
fn check_fork_team(
    state: &AppState,
    parent: &Team,
    changes: &HashMap<String, Change>,
) -> Result<(), ApiError> {
    let team = fork::apply_changes(parent, changes);
    let changed_open = team
        .agents
        .iter()
        .filter(|a| a.kind == AgentKind::Open && changes.contains_key(&a.id));
    for a in changed_open {
        open_check(state, a.provider.as_deref(), a.model.as_deref())?;
    }
    Ok(())
}

/// A blank provider means the default one, as in a fork change.
fn settle_providers(team: &mut Team) {
    for a in &mut team.agents {
        a.provider = a.provider.as_deref().and_then(trimmed);
    }
}

fn validate_team(state: &AppState, team: &Team) -> Result<(), ApiError> {
    if team.agents.is_empty() {
        return Err(ApiError::bad("the team needs at least one agent"));
    }
    validate_agents(state, &team.agents)
}

/// 400 unless arenad can run the team itself under `runner`: some agents, each valid, none
/// external; in a dispatch run at most one lead, and only agents that keep a session.
fn validate_builtin_team(state: &AppState, team: &Team, runner: Runner) -> Result<(), ApiError> {
    validate_team(state, team)?;
    if team.agents.iter().any(|a| a.kind == AgentKind::External) {
        return Err(ApiError::bad(
            "an external agent runs only in a run with runner \"external\"",
        ));
    }
    match runner {
        Runner::Dispatch => validate_dispatch_team(team),
        _ => Ok(()),
    }
}

/// 400 when two agents are marked `lead`, or an agent can't stay alive across turns (Codex).
fn validate_dispatch_team(team: &Team) -> Result<(), ApiError> {
    if team.agents.iter().filter(|a| a.lead).count() > 1 {
        return Err(ApiError::bad("mark at most one agent as the lead"));
    }
    if let Some(a) = team.agents.iter().find(|a| a.kind == AgentKind::Codex) {
        return Err(ApiError::bad(format!(
            "agent {:?} is a Codex agent, which can't keep a session across turns yet: a dispatch run needs Claude Code or arenad's own loop (open)",
            a.id
        )));
    }
    Ok(())
}

/// 400 unless every agent has a good, unique id and an open agent can run.
fn validate_agents(state: &AppState, agents: &[AgentSpec]) -> Result<(), ApiError> {
    let mut seen = std::collections::HashSet::new();
    for a in agents {
        validate_agent_id(&a.id)?;
        if !seen.insert(a.id.as_str()) {
            return Err(ApiError::bad(format!("duplicate agent id {:?}", a.id)));
        }
        if a.kind == AgentKind::Open {
            open_check(state, a.provider.as_deref(), a.model.as_deref())?;
        }
    }
    Ok(())
}

/// 400 unless `id` matches `^[a-z0-9_-]+$` and isn't the links' actor.
fn validate_agent_id(id: &str) -> Result<(), ApiError> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !ok {
        return Err(ApiError::bad(format!(
            "agent id {id:?} must match ^[a-z0-9_-]+$"
        )));
    }
    if id == crate::exec::LINK_ACTOR || id == crate::exec::SETUP_ACTOR {
        return Err(ApiError::bad(format!(
            "agent id {id:?} is kept for steps the world makes (its links, a task's setup)"
        )));
    }
    Ok(())
}

/// Runs `f` on its own task and waits for it, so a client disconnect can't cancel it midway.
async fn detached<T: Send + 'static>(
    f: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::spawn(f).await.map_err(|e| {
        ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("request task failed: {e}"),
        )
    })?
}

// ---------------------------------------------------------------- simple routes

pub fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p).any(|d| {
            let f = d.join(name);
            f.is_file() && {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::metadata(&f).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
                }
                #[cfg(not(unix))]
                {
                    true
                }
            }
        })
    })
}

async fn health(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bytes = gc::usage(&state.hub, &state.benchmark).await?;
    let benchmark = match state.bench().await {
        Ok(b) => json!({"name": b.name, "version": b.version, "tasks": b.tasks.len(),
                        "world": b.world.iter().map(|p| json!({"part": p.part, "integration": p.integration})).collect::<Vec<_>>()}),
        Err(e) => json!({"name": state.benchmark, "error": format!("{e:#}")}),
    };
    Ok(Json(json!({
        "open_available": state.config.open_available(),
        "providers": providers::summary(&state.config.providers),
        "claude": on_path("claude"),
        "codex": on_path("codex"),
        "benchmark": benchmark,
        "disk": {"bytes": bytes, "limit_bytes": state.config.max_db_bytes},
    })))
}

/// `[{id, kind, label, available, why?, models?}]`: each provider, asking the ones that list
/// their models (each bounded by `providers::LIST_TIMEOUT`, all at once).
async fn list_providers(State(state): State<Arc<AppState>>) -> Json<Vec<ProviderStatus>> {
    Json(providers::statuses(&state.config.providers, &state.http).await)
}

/// 507 when the arena's saves already use at least the configured limit.
async fn check_disk(state: &AppState) -> Result<(), ApiError> {
    if state.saves_over_limit().await? {
        return Err(ApiError::insufficient_storage(SAVES_FULL));
    }
    Ok(())
}

/// Why a run is refused while the saves are over the limit.
pub const SAVES_FULL: &str = "Shakedown's saves use more than the limit; delete old runs";

/// True when any branch of the run is running (stored as running, or live in this process).
async fn run_is_running(state: &AppState, id: &str) -> Result<bool, ApiError> {
    if state.store.run_has_running(id).await? {
        return Ok(true);
    }
    let branches = state.store.run_branches(id).await?;
    let live = state.live.read().unwrap();
    Ok(branches.iter().any(|b| live.contains_key(b)))
}

async fn delete_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    if !state.store.run_exists(&id).await? {
        return Err(ApiError::not_found(format!("there is no run \"{id}\"")));
    }
    if run_is_running(&state, &id).await? {
        return Err(ApiError::conflict("the run still has a running branch"));
    }
    detached(async move {
        let dropped = gc::delete_run(&state.store, &state.hub, &id).await?;
        Ok(Json(json!({"dropped": dropped})))
    })
    .await
}

/// The benchmark's tasks: `[{id, reason, ticket, instructions}]`; an external design gives its
/// agents the ticket and the instructions as arenad's own runners do.
async fn tasks(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    let task = |t: &parts::proto::TaskInfo| {
        json!({"id": t.id, "reason": t.reason, "ticket": t.ticket,
               "instructions": bench.instructions})
    };
    Ok(Json(bench.tasks.iter().map(task).collect()))
}

/// The team the benchmark suggests, or arenad's generic one; `kind`, `model` and `provider`
/// in the query set every agent's (Claude Code with `sonnet` when absent).
async fn default_team(
    State(state): State<Arc<AppState>>,
    Query(q): Query<prompts::Pick>,
) -> ApiResult<Team> {
    Ok(Json(prompts::default_team_with(&state.bench().await?, &q)))
}

/// Every team the benchmark suggests, the default first, each with the runner it is written for
/// (`{...team, runner}`); the query as for `default_team`.
async fn suggested_teams(
    State(state): State<Arc<AppState>>,
    Query(q): Query<prompts::Pick>,
) -> ApiResult<Vec<prompts::Suggested>> {
    Ok(Json(prompts::suggested_teams(&state.bench().await?, &q)))
}

async fn list_runs(State(state): State<Arc<AppState>>) -> ApiResult<Vec<RunSummary>> {
    Ok(Json(state.store.runs().await?))
}

// ---------------------------------------------------------------- runs

#[derive(Deserialize)]
struct NewRun {
    task_id: String,
    team: Team,
    /// `chain` (default), `concurrent`, `dispatch`, or `external` (see `api::external`).
    #[serde(default)]
    runner: Runner,
    /// The disruptions to add to the task (`{disruptions: [{id, params}]}`).
    #[serde(default)]
    scenario: Option<parts::proto::ScenarioRequest>,
    /// The world's seed (at most 2^53 - 1); random when absent.
    #[serde(default)]
    seed: Option<u64>,
}

async fn create_run(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Value> {
    let cleanup = not_cleaning(&state)?;
    let mut req: NewRun = parse_body(&body)?;
    settle_providers(&mut req.team);
    let mut scenario = twin::scenario_of(&state, req.scenario.as_ref()).await?;
    if req.seed.is_some_and(|s| s > runner::MAX_SEED) {
        return Err(ApiError::bad("seed must be at most 2^53 - 1"));
    }
    scenario.seed = req.seed;
    if req.runner == Runner::External {
        return external::create_run(state, cleanup, req.task_id, req.team, scenario).await;
    }
    validate_builtin_team(&state, &req.team, req.runner)?;
    check_disk(&state).await?;
    let (run_id, branch_id) = detached(async move {
        let _cleanup = cleanup;
        let (task, team) = (&req.task_id, req.team);
        Ok(runner::start_run_in(&state, task, team, req.runner, &scenario).await?)
    })
    .await?;
    Ok(Json(json!({"run_id": run_id, "branch_id": branch_id})))
}

async fn get_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let branch = branch_or_404(&state, &id).await?;
    let events = state.store.events(&id).await?;
    let now = chrono::Utc::now();
    let m = metrics::metrics(&branch, &events, prices::table(), now);
    let mut v = serde_json::to_value(branch).map_err(anyhow::Error::from)?;
    v["events"] = serde_json::to_value(events).map_err(anyhow::Error::from)?;
    v["metrics"] = serde_json::to_value(m).map_err(anyhow::Error::from)?;
    v["paused"] = json!(flow::is_paused(&state, &id));
    Ok(Json(v))
}

fn exact() -> Mode {
    Mode::Exact
}

#[derive(Deserialize)]
struct ForkReq {
    at_seq: i64,
    #[serde(default)]
    changes: HashMap<String, Change>,
    /// `"exact"` (default) or `"approximate"`.
    #[serde(default = "exact")]
    restore: Mode,
    /// Another team, for a run an external system drives (in place of `changes`).
    #[serde(default)]
    team: Option<Team>,
}

/// The branch to fork: 404 for an unknown branch and 409 for a running one; these come before
/// the 507 check.
async fn forkable(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    match state.store.find_branch(id).await? {
        None => Err(no_branch(id)),
        Some(b) if b.status == BranchStatus::Running => {
            Err(ApiError::conflict("the branch is still running"))
        }
        Some(b) => Ok(b),
    }
}

async fn fork_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<fork::ForkResponse> {
    let cleanup = not_cleaning(&state)?;
    let mut req: ForkReq = parse_body(&body)?;
    let parent = forkable(&state, &id).await?;
    fork::settle_changes(&parent.team, &mut req.changes, &state.config.providers);
    check_fork_team(&state, &parent.team, &req.changes)?;
    if let Some(team) = &mut req.team {
        settle_providers(team);
        validate_team(&state, team)?;
    }
    check_disk(&state).await?;
    let req = fork::ForkRequest {
        at_seq: req.at_seq,
        changes: req.changes,
        restore: req.restore,
        team: req.team,
    };
    let resp = detached(async move {
        let _cleanup = cleanup;
        Ok(fork::fork_with(&state, &id, req).await?)
    })
    .await?;
    Ok(Json(resp))
}

/// `restore=exact|approximate` (default exact).
fn parse_restore(q: &HashMap<String, String>) -> Result<Mode, ApiError> {
    match q.get("restore").filter(|r| !r.is_empty()) {
        None => Ok(exact()),
        Some(r) => serde_json::from_value(json!(r))
            .map_err(|_| ApiError::bad(format!("restore must be exact or approximate, not {r:?}"))),
    }
}

/// `{parts: [{part, how, estimate_seconds?}]}`: how each part of a fork at `at_seq` would be
/// restored, or the refusal (409).
async fn get_restore_plan(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let at_seq: i64 = q
        .get("at_seq")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::bad("at_seq must be an integer"))?;
    let mode = parse_restore(&q)?;
    let parts =
        detached(async move { Ok(fork::restore_preview(&state, &id, at_seq, mode).await?) })
            .await?;
    Ok(Json(json!({"parts": parts})))
}
