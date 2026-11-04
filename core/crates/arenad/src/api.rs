//! The HTTP API the UI talks to. All JSON except the SSE stream.

use std::{
    collections::HashMap,
    path::{Component, Path as FsPath, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, Method, StatusCode, Uri},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use parts::proto::{Mode, WorldPart};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{
    broadcast::{self, error::RecvError},
    mpsc, OwnedRwLockReadGuard,
};

use crate::{
    app::AppState,
    compare::{self, FinalState},
    fork::{self, ForkError},
    gc, metrics,
    model::*,
    prices, prompts,
    providers::{self, ProviderStatus},
    recorded::{final_saves, run_info, saves_at},
    runner,
    saves::{self, part_code, part_named, part_refusal, world_of},
};

mod external;
mod world;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/providers", get(list_providers))
        .route("/api/tasks", get(tasks))
        .route("/api/team/default", get(default_team))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/{id}", axum::routing::delete(delete_run))
        .route("/api/branches/{id}", get(get_branch))
        .route("/api/branches/{id}/stream", get(stream))
        .route("/api/branches/{id}/rows", get(rows))
        .route("/api/branches/{id}/fork", post(fork_branch))
        .route("/api/branches/{id}/restore-plan", get(get_restore_plan))
        .route("/api/branches/{id}/agents", post(external::register))
        .route(
            "/api/branches/{id}/agents/{agent}/end",
            post(external::end_agent),
        )
        .route("/api/branches/{id}/finish", post(external::finish))
        .route("/api/compare", get(compare_branches))
        .route("/api/world", get(world::world))
        .route("/api/usage", get(world::usage))
        .route("/api/gc", post(world::gc))
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

fn validate_team(state: &AppState, team: &Team) -> Result<(), ApiError> {
    if team.agents.is_empty() {
        return Err(ApiError::bad("the team needs at least one agent"));
    }
    validate_agents(state, &team.agents)
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
    if id == crate::exec::LINK_ACTOR {
        return Err(ApiError::bad(format!(
            "agent id {id:?} is kept for steps made by a world's links"
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
    let bytes = gc::usage(&state.hub, &state.benchmark).await?;
    if bytes >= state.config.max_db_bytes {
        return Err(ApiError::insufficient_storage(
            "the arena's saves use more than the limit; delete old runs",
        ));
    }
    Ok(())
}

/// True when any branch of the run is running (stored as running, or live in this process).
async fn run_is_running(state: &AppState, id: &str) -> Result<bool, ApiError> {
    if state.store.run_has_running(id).await? {
        return Ok(true);
    }
    let live = state
        .live
        .read()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    Ok(state
        .store
        .runs()
        .await?
        .iter()
        .filter(|r| r.id == id)
        .flat_map(|r| &r.branches)
        .any(|b| live.contains(&b.id)))
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

/// The benchmark's tasks: `[{id, reason}]`.
async fn tasks(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    let bench = state.bench().await?;
    Ok(Json(
        bench
            .tasks
            .iter()
            .map(|t| json!({"id": t.id, "reason": t.reason}))
            .collect(),
    ))
}

/// The team the benchmark suggests, or arenad's generic one; `kind`, `model` and `provider`
/// in the query set every agent's (Claude Code with `sonnet` when absent).
async fn default_team(
    State(state): State<Arc<AppState>>,
    Query(q): Query<prompts::Pick>,
) -> ApiResult<Team> {
    Ok(Json(prompts::default_team_with(&state.bench().await?, &q)))
}

async fn list_runs(State(state): State<Arc<AppState>>) -> ApiResult<Vec<RunSummary>> {
    Ok(Json(state.store.runs().await?))
}

// ---------------------------------------------------------------- runs

#[derive(Deserialize)]
struct NewRun {
    task_id: String,
    team: Team,
    /// `chain` (default), `concurrent`, or `external` (see `api::external`).
    #[serde(default)]
    runner: Runner,
}

async fn create_run(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Value> {
    let cleanup = not_cleaning(&state)?;
    let mut req: NewRun = parse_body(&body)?;
    // A blank provider means the default one, as in a fork change.
    for a in &mut req.team.agents {
        a.provider = a.provider.as_deref().and_then(trimmed);
    }
    if req.runner == Runner::External {
        return external::create_run(state, cleanup, req.task_id, req.team).await;
    }
    validate_team(&state, &req.team)?;
    if req
        .team
        .agents
        .iter()
        .any(|a| a.kind == AgentKind::External)
    {
        return Err(ApiError::bad(
            "an external agent runs only in a run with runner \"external\"",
        ));
    }
    check_disk(&state).await?;
    let (run_id, branch_id) = detached(async move {
        let _cleanup = cleanup;
        let (task, team) = (&req.task_id, req.team);
        Ok(runner::start_run_with(&state, task, team, req.runner).await?)
    })
    .await?;
    Ok(Json(json!({"run_id": run_id, "branch_id": branch_id})))
}

async fn get_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Value> {
    let branch = state
        .store
        .find_branch(&id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("there is no branch \"{id}\"")))?;
    let events = state.store.events(&id).await?;
    let now = chrono::Utc::now();
    let m = metrics::metrics(&branch, &events, prices::table(), now);
    let mut v = serde_json::to_value(branch).map_err(anyhow::Error::from)?;
    v["events"] = serde_json::to_value(events).map_err(anyhow::Error::from)?;
    v["metrics"] = serde_json::to_value(m).map_err(anyhow::Error::from)?;
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
}

/// The branch to fork: 404 for an unknown branch and 409 for a running one; these come before
/// the 507 check.
async fn forkable(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    match state.store.find_branch(id).await? {
        None => Err(ApiError::not_found(format!("there is no branch \"{id}\""))),
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
    check_disk(&state).await?;
    let req = fork::ForkRequest {
        at_seq: req.at_seq,
        changes: req.changes,
        restore: req.restore,
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

// ---------------------------------------------------------------- SSE

fn sse_event(name: &str, data: &impl serde::Serialize) -> SseEvent {
    SseEvent::default()
        .event(name)
        .json_data(data)
        .unwrap_or_else(|_| SseEvent::default().event(name).data("null"))
}

type SseTx = mpsc::Sender<Result<SseEvent, std::convert::Infallible>>;

/// Feeds one branch's events and final status to an SSE client.
struct Pump {
    state: Arc<AppState>,
    id: String,
    /// The last event seq sent.
    last: i64,
    tx: SseTx,
}

/// What woke the pump.
enum Wake {
    Stop,
    Tick,
    Ev(Result<Event, RecvError>),
}

/// What the event pump does after a wake-up.
enum Next {
    /// Stop: arenad shuts down, or the client or the store is gone.
    Stop,
    /// See whether the branch has ended, then wait.
    CheckStatus,
    /// Wait for the next wake-up.
    Wait,
}

/// `then` when `ok`, else `Next::Stop`.
fn unless_gone(ok: bool, then: Next) -> Next {
    if ok {
        then
    } else {
        Next::Stop
    }
}

impl Pump {
    /// Sends the stored events after `last`; false when the client is gone or the store failed.
    async fn catch_up(&mut self) -> bool {
        let Ok(events) = self.state.store.events(&self.id).await else {
            return false;
        };
        let from = self.last;
        for e in events.into_iter().filter(|e| e.seq > from) {
            self.last = e.seq;
            if self.tx.send(Ok(sse_event("event", &e))).await.is_err() {
                return false;
            }
        }
        true
    }

    /// Handles one wake-up.
    async fn on_wake(&mut self, wake: Wake) -> Next {
        match wake {
            Wake::Stop => Next::Stop,
            Wake::Tick => Next::CheckStatus,
            Wake::Ev(Ok(e)) if e.branch_id == self.id => self.on_event(&e).await,
            Wake::Ev(Ok(_)) => Next::Wait,
            Wake::Ev(Err(RecvError::Lagged(_))) => {
                unless_gone(self.catch_up().await, Next::CheckStatus)
            }
            Wake::Ev(Err(RecvError::Closed)) => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                Next::Wait
            }
        }
    }

    /// An event of this branch: sent when it is the next one; after a gap, the missed ones are
    /// read from the store.
    async fn on_event(&mut self, e: &Event) -> Next {
        let ok = if e.seq == self.last + 1 {
            self.last = e.seq;
            self.tx.send(Ok(sse_event("event", e))).await.is_ok()
        } else {
            e.seq <= self.last || self.catch_up().await
        };
        unless_gone(ok, Next::Wait)
    }

    /// Sends the final `status` once the branch is no longer running; false to stop.
    async fn status_check(&mut self) -> bool {
        let Ok(Some(b)) = self.state.store.find_branch(&self.id).await else {
            return false;
        };
        if b.status == BranchStatus::Running {
            return true;
        }
        // Events written just before the status flipped may not have been seen yet.
        if self.catch_up().await {
            let status = json!({"status": b.status, "error": b.error});
            let _ = self.tx.send(Ok(sse_event("status", &status))).await;
        }
        false
    }

    async fn run(mut self, mut rx: broadcast::Receiver<Event>) {
        let mut shutdown = self.state.shutdown.subscribe();
        if *shutdown.borrow() || !self.catch_up().await {
            return;
        }
        // Status is checked on its own timer so other branches' events can't starve it,
        // and once right away so a finished branch gets its `status` immediately.
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            let wake = tokio::select! {
                _ = shutdown.changed() => Wake::Stop,
                _ = tick.tick() => Wake::Tick,
                got = rx.recv() => Wake::Ev(got),
            };
            match self.on_wake(wake).await {
                Next::Stop => return,
                Next::CheckStatus if !self.status_check().await => return,
                Next::CheckStatus | Next::Wait => {}
            }
        }
    }
}

async fn stream(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    if state.store.find_branch(&id).await?.is_none() {
        return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
    }
    // Subscribe before reading the stored events so nothing falls between the two.
    let rx = state.store.subscribe();
    let (tx, out) = mpsc::channel(64);
    tokio::spawn(
        Pump {
            state,
            id,
            last: 0,
            tx,
        }
        .run(rx),
    );
    let stream =
        futures_util::stream::unfold(out, |mut rx| async { rx.recv().await.map(|x| (x, rx)) });
    Ok(Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response())
}

// ---------------------------------------------------------------- rows

const MAX_KEYS: usize = 50;

/// A rows request: the step, at most `MAX_KEYS` keys (`<kind>/<id>`), and the part to read.
struct RowsQuery {
    seq: i64,
    keys: Vec<String>,
    part: Option<String>,
}

fn parse_rows_query(q: &HashMap<String, String>) -> Result<RowsQuery, ApiError> {
    let seq: i64 = q
        .get("seq")
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| ApiError::bad("seq must be an integer"))?;
    let keys: Vec<String> = q
        .get("keys")
        .map(String::as_str)
        .unwrap_or("")
        .split(',')
        .filter(|k| !k.is_empty())
        .map(|k| match k.split_once('/') {
            Some((kind, id)) if !kind.is_empty() && !id.is_empty() => Ok(k.to_string()),
            _ => Err(ApiError::bad(format!(
                "bad key {k:?}: expected <kind>/<id>"
            ))),
        })
        .collect::<Result<_, _>>()?;
    if keys.len() > MAX_KEYS {
        return Err(ApiError::bad(format!(
            "at most {MAX_KEYS} keys per request"
        )));
    }
    let part = q.get("part").filter(|p| !p.is_empty()).cloned();
    Ok(RowsQuery { seq, keys, part })
}

/// The part a rows request reads: `?part=`, else the step's part, else the world's first part.
fn rows_part<'a>(
    asked: Option<&str>,
    step: &Event,
    world: &'a [WorldPart],
) -> Result<&'a WorldPart, ApiError> {
    match asked.or(step.payload["part"].as_str()) {
        Some(n) => part_named(world, n)
            .ok_or_else(|| ApiError::bad(format!("there is no part {n} in this world"))),
        None => world
            .first()
            .ok_or_else(|| ApiError::bad("this branch's world has no parts")),
    }
}

/// A part's refusal of the keys (`-32602`) is a bad request with the part's message; a save that
/// is gone (`-32002`) a conflict.
fn rows_error(e: anyhow::Error) -> ApiError {
    match part_refusal(&e) {
        Some((-32602, message)) => ApiError::bad(message),
        Some((-32002, _)) => ApiError::conflict("the saved state for that step is gone"),
        _ => e.into(),
    }
}

/// The docs of the request's keys in the save holding the part's state just after the step.
async fn read_rows(state: &AppState, events: &[Event], q: &RowsQuery) -> Result<Value, ApiError> {
    let step = events
        .iter()
        .find(|e| e.seq == q.seq)
        .ok_or_else(|| ApiError::bad("there is no step with that number on this branch"))?;
    let saves = saves_at(events, q.seq, &run_info(events).base).map_err(|n| {
        ApiError::bad(format!(
            "the world after that step can't be restored: the write at step {} to the part {} has no save",
            n.seq, n.part
        ))
    })?;
    let world = world_of(&state.hub, events).await?;
    let part = rows_part(q.part.as_deref(), step, &world)?;
    let save = saves
        .get(&part.part)
        .ok_or_else(|| ApiError::bad(format!("there is no save of the part {}", part.part)))?;
    let docs = saves::inspect(&state.hub, &part.integration, save, &q.keys)
        .await
        .map_err(rows_error)?;
    Ok(json!({"seq": q.seq, "rows": docs}))
}

async fn rows(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let q = parse_rows_query(&q)?;
    if state.store.find_branch(&id).await?.is_none() {
        return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
    }
    let events = state.store.events(&id).await?;
    Ok(Json(read_rows(&state, &events, &q).await?))
}

// ---------------------------------------------------------------- compare

const NO_FINAL_STATE: &str = "this branch has no final state yet";

/// The branch, which must exist (404) and have finished (409).
async fn finished_branch(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    let b = state
        .store
        .find_branch(id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("there is no branch \"{id}\"")))?;
    if b.status == BranchStatus::Running {
        return Err(ApiError::conflict(format!(
            "branch \"{id}\" is still running"
        )));
    }
    Ok(b)
}

/// One side of a comparison: the branch, its events and its final state (409 without one).
struct Side {
    branch: Branch,
    events: Vec<Event>,
    end: FinalState,
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
async fn sides(state: &AppState, ids: [&String; 2]) -> Result<[Side; 2], ApiError> {
    for id in ids {
        if state.store.find_branch(id).await?.is_none() {
            return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
        }
    }
    let (a, b) = (
        finished_branch(state, ids[0]).await?,
        finished_branch(state, ids[1]).await?,
    );
    Ok([side(state, a).await?, side(state, b).await?])
}

async fn compare_branches(
    State(state): State<Arc<AppState>>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let (a, b) = match (q.get("a"), q.get("b")) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(ApiError::bad("a and b are required")),
    };
    let [a, b] = sides(&state, [a, b]).await?;
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
async fn compared_rows(
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

// ---------------------------------------------------------------- static UI

fn content_type(p: &FsPath) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Serves `dist` for GET/HEAD requests the API doesn't claim; unknown non-asset paths get
/// `index.html` (SPA). `/api/`, `/mcp/`, `/gateway/` and missing `/assets/` files are 404s.
pub fn with_ui(router: Router, dist: PathBuf) -> Router {
    router.fallback(move |method: Method, uri: Uri| {
        let dist = dist.clone();
        async move {
            let path = uri.path();
            let reserved = ["/api/", "/mcp/", "/gateway/"]
                .iter()
                .any(|p| path.starts_with(p) || path == p.trim_end_matches('/'));
            if reserved || !(method == Method::GET || method == Method::HEAD) {
                return ApiError::not_found("no such route").into_response();
            }
            let rel = path.trim_start_matches('/');
            let safe = FsPath::new(rel)
                .components()
                .all(|c| matches!(c, Component::Normal(_)));
            let candidate = dist.join(rel);
            let file = if safe && !rel.is_empty() && candidate.is_file() {
                candidate
            } else if rel.starts_with("assets/") || !safe && rel.contains("assets/") {
                return ApiError::not_found("no such file").into_response();
            } else {
                dist.join("index.html")
            };
            match tokio::fs::read(&file).await {
                Ok(bytes) => ([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response(),
                Err(_) => StatusCode::NOT_FOUND.into_response(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    async fn call(app: &Router, method: &str, uri: &str) -> (StatusCode, String, String) {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let ct = r
            .headers()
            .get(header::CONTENT_TYPE)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let st = r.status();
        let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (st, ct, String::from_utf8_lossy(&body).to_string())
    }

    /// A scratch folder with a built UI (`dist/`) and a secret beside it; returns the folder.
    fn ui_folder() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("arena_ui_{}", uuid::Uuid::new_v4().simple()));
        let dist = root.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), "INDEX").unwrap();
        std::fs::write(dist.join("assets/a.js"), "JS").unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        root
    }

    /// Paths that climb out of the UI folder never serve the secret.
    async fn assert_no_path_escape(app: &Router) {
        for p in [
            "/../secret.txt",
            "/%2e%2e/secret.txt",
            "/assets/../../secret.txt",
            "/%2e%2e%2fsecret.txt",
        ] {
            let (_, _, body) = call(app, "GET", p).await;
            assert!(!body.contains("SECRET"), "{p} escaped");
        }
    }

    #[tokio::test]
    async fn spa_fallback_is_scoped() {
        let root = ui_folder();
        let app = with_ui(Router::new(), root.join("dist"));

        assert_eq!(call(&app, "GET", "/").await.2, "INDEX");
        assert_eq!(call(&app, "GET", "/runs/x").await.2, "INDEX");
        let (st, ct, body) = call(&app, "GET", "/assets/a.js").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "JS"));
        assert!(ct.starts_with("text/javascript"));
        assert_eq!(
            call(&app, "GET", "/assets/missing.js").await.0,
            StatusCode::NOT_FOUND
        );
        let (st, ct, body) = call(&app, "POST", "/mcp/nope").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(
            ct.starts_with("application/json") && body.contains("error"),
            "{ct} {body}"
        );
        assert_eq!(
            call(&app, "GET", "/gateway/x").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(call(&app, "GET", "/api/x").await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "POST", "/runs/x").await.0, StatusCode::NOT_FOUND);
        assert_no_path_escape(&app).await;
        let _ = std::fs::remove_dir_all(root);
    }
}
