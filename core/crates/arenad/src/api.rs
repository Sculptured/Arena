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
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use world::snapshot::Source;

use crate::{
    app::AppState,
    compare,
    fork::{self, ForkError},
    model::*,
    prompts, runner,
};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/tasks", get(tasks))
        .route("/api/team/default", get(default_team))
        .route("/api/runs", get(list_runs).post(create_run))
        .route("/api/runs/{id}", axum::routing::delete(delete_run))
        .route("/api/branches/{id}", get(get_branch))
        .route("/api/branches/{id}/stream", get(stream))
        .route("/api/branches/{id}/rows", get(rows))
        .route("/api/branches/{id}/fork", post(fork_branch))
        .route("/api/compare", get(compare_branches))
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
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl From<ForkError> for ApiError {
    fn from(e: ForkError) -> ApiError {
        let status = match &e {
            ForkError::UnknownBranch(_) => StatusCode::NOT_FOUND,
            ForkError::BranchRunning => StatusCode::CONFLICT,
            ForkError::UnknownSeq | ForkError::UnknownAgent(_) | ForkError::MissingHandoff(_) | ForkError::NoRestorePoint { .. } => {
                StatusCode::BAD_REQUEST
            }
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

fn open_check(state: &AppState) -> Result<(), ApiError> {
    if state.config.open_available() {
        Ok(())
    } else {
        Err(ApiError::bad("open mode needs OPENROUTER_API_KEY"))
    }
}

fn validate_team(state: &AppState, team: &Team) -> Result<(), ApiError> {
    if team.agents.is_empty() {
        return Err(ApiError::bad("the team needs at least one agent"));
    }
    let mut seen = std::collections::HashSet::new();
    for a in &team.agents {
        let ok = !a.id.is_empty() && a.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
        if !ok {
            return Err(ApiError::bad(format!("agent id {:?} must match ^[a-z0-9_-]+$", a.id)));
        }
        if !seen.insert(a.id.as_str()) {
            return Err(ApiError::bad(format!("duplicate agent id {:?}", a.id)));
        }
        if a.kind == AgentKind::Open {
            open_check(state)?;
        }
    }
    Ok(())
}

/// Runs `f` on its own task and waits for it, so a client disconnect can't cancel it midway.
async fn detached<T: Send + 'static>(f: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static) -> Result<T, ApiError> {
    tokio::spawn(f).await.map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("request task failed: {e}")))?
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
    let (databases, bytes) = crate::gc::usage(&state.worlds).await?;
    Ok(Json(json!({
        "open_available": state.config.open_available(),
        "claude": on_path("claude"),
        "codex": on_path("codex"),
        "disk": {"databases": databases, "bytes": bytes, "limit_bytes": state.config.max_db_bytes},
    })))
}

/// 507 when the arena's databases already use at least the configured limit.
async fn check_disk(state: &AppState) -> Result<(), ApiError> {
    let (_, bytes) = crate::gc::usage(&state.worlds).await?;
    if bytes as u64 >= state.config.max_db_bytes {
        return Err(ApiError::insufficient_storage("the arena's databases use more than the limit; delete old runs"));
    }
    Ok(())
}

async fn delete_run(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    if !state.store.run_exists(&id).await? {
        return Err(ApiError::not_found(format!("there is no run \"{id}\"")));
    }
    let live = state.live.read().unwrap().keys().cloned().collect::<Vec<_>>();
    let running = state.store.run_has_running(&id).await?
        || state.store.runs().await?.iter().filter(|r| r.id == id).flat_map(|r| &r.branches).any(|b| live.contains(&b.id));
    if running {
        return Err(ApiError::conflict("the run still has a running branch"));
    }
    detached(async move {
        let dbs = state.store.delete_run(&id).await?;
        let mut dropped = 0;
        for db in &dbs {
            match state.worlds.drop_db(db).await {
                Ok(()) => dropped += 1,
                Err(e) => eprintln!("delete run {id}: could not drop {db}: {e:#}"),
            }
        }
        state.dumps.lock().unwrap().retain(|b, _| !dbs.contains(&format!("branch_{b}")));
        Ok(Json(json!({"dropped": dropped})))
    })
    .await
}

async fn tasks(State(state): State<Arc<AppState>>) -> ApiResult<Value> {
    Ok(Json(serde_json::to_value(state.tasks()?.demo()).map_err(anyhow::Error::from)?))
}

async fn default_team() -> Json<Team> {
    Json(prompts::default_team())
}

async fn list_runs(State(state): State<Arc<AppState>>) -> ApiResult<Vec<RunSummary>> {
    Ok(Json(state.store.runs().await?))
}

// ---------------------------------------------------------------- runs

#[derive(Deserialize)]
struct NewRun {
    task_id: String,
    team: Team,
}

async fn create_run(State(state): State<Arc<AppState>>, body: Bytes) -> ApiResult<Value> {
    let req: NewRun = parse_body(&body)?;
    if state.tasks()?.get(&req.task_id).is_none() {
        return Err(ApiError::not_found(format!("unknown task {}", req.task_id)));
    }
    validate_team(&state, &req.team)?;
    check_disk(&state).await?;
    let (run_id, branch_id) = detached(async move { Ok(runner::start_run(&state, &req.task_id, req.team).await?) }).await?;
    Ok(Json(json!({"run_id": run_id, "branch_id": branch_id})))
}

async fn get_branch(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let branch = state.store.find_branch(&id).await?.ok_or_else(|| ApiError::not_found(format!("there is no branch \"{id}\"")))?;
    let events = state.store.events(&id).await?;
    let mut v = serde_json::to_value(branch).map_err(anyhow::Error::from)?;
    v["events"] = serde_json::to_value(events).map_err(anyhow::Error::from)?;
    Ok(Json(v))
}

#[derive(Deserialize)]
struct ForkReq {
    at_seq: i64,
    #[serde(default)]
    changes: HashMap<String, Change>,
}

async fn fork_branch(State(state): State<Arc<AppState>>, Path(id): Path<String>, body: Bytes) -> ApiResult<fork::ForkResponse> {
    let req: ForkReq = parse_body(&body)?;
    if req.changes.values().any(|c| c.kind == Some(AgentKind::Open)) {
        open_check(&state)?;
    }
    check_disk(&state).await?;
    let resp = detached(async move { Ok(fork::fork(&state, &id, req.at_seq, req.changes).await?) }).await?;
    Ok(Json(resp))
}

// ---------------------------------------------------------------- SSE

fn sse_event(name: &str, data: &impl serde::Serialize) -> SseEvent {
    SseEvent::default().event(name).json_data(data).unwrap_or_else(|_| SseEvent::default().event(name).data("null"))
}

async fn stream(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Response, ApiError> {
    if state.store.find_branch(&id).await?.is_none() {
        return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
    }
    // Subscribe before reading the stored events so nothing falls between the two.
    let mut rx = state.store.subscribe();
    let (tx, out) = mpsc::channel::<Result<SseEvent, std::convert::Infallible>>(64);
    tokio::spawn(async move {
        let mut last = 0i64;
        // Sends the stored events after `last`; false when the client is gone or the store failed.
        async fn catch_up(state: &AppState, id: &str, last: &mut i64, tx: &mpsc::Sender<Result<SseEvent, std::convert::Infallible>>) -> bool {
            let Ok(events) = state.store.events(id).await else { return false };
            let from = *last;
            for e in events.into_iter().filter(|e| e.seq > from) {
                *last = e.seq;
                if tx.send(Ok(sse_event("event", &e))).await.is_err() {
                    return false;
                }
            }
            true
        }
        let mut shutdown = state.shutdown.subscribe();
        if *shutdown.borrow() {
            return;
        }
        if !catch_up(&state, &id, &mut last, &tx).await {
            return;
        }
        // Status is checked on its own timer so other branches' events can't starve it,
        // and once right away so a finished branch gets its `status` immediately.
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            enum Wake {
                Stop,
                Tick,
                Ev(Result<Event, RecvError>),
            }
            let wake = tokio::select! {
                _ = shutdown.changed() => Wake::Stop,
                _ = tick.tick() => Wake::Tick,
                got = rx.recv() => Wake::Ev(got),
            };
            let mut check_status = false;
            match wake {
                Wake::Stop => return,
                Wake::Tick => check_status = true,
                Wake::Ev(Ok(e)) if e.branch_id == id => {
                    if e.seq == last + 1 {
                        last = e.seq;
                        if tx.send(Ok(sse_event("event", &e))).await.is_err() {
                            return;
                        }
                    } else if e.seq > last && !catch_up(&state, &id, &mut last, &tx).await {
                        return;
                    }
                }
                Wake::Ev(Ok(_)) => {}
                Wake::Ev(Err(RecvError::Lagged(_))) => {
                    if !catch_up(&state, &id, &mut last, &tx).await {
                        return;
                    }
                    check_status = true;
                }
                Wake::Ev(Err(RecvError::Closed)) => tokio::time::sleep(Duration::from_millis(250)).await,
            }
            if check_status {
                let Ok(Some(b)) = state.store.find_branch(&id).await else { return };
                if b.status != BranchStatus::Running {
                    // Events written just before the status flipped may not have been seen yet.
                    if !catch_up(&state, &id, &mut last, &tx).await {
                        return;
                    }
                    let _ = tx.send(Ok(sse_event("status", &json!({"status": b.status, "error": b.error})))).await;
                    return;
                }
            }
        }
    });
    let stream = futures_util::stream::unfold(out, |mut rx| async { rx.recv().await.map(|x| (x, rx)) });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))).into_response())
}

// ---------------------------------------------------------------- rows

const MAX_KEYS: usize = 50;

/// Where the world stood just after step `seq`: base, or the last snapshot at or before it.
fn restore_source(events: &[Event], seq: i64) -> Result<Source, ApiError> {
    let mut src = Some(Source::Base);
    for e in events.iter().filter(|e| e.seq <= seq && e.kind == EventKind::ToolCall) {
        if let Some(s) = e.payload["snapshot"].as_str() {
            src = Some(Source::Snapshot(s.to_string()));
        } else if e.payload["error"].is_null() && e.payload["writes"].as_array().is_some_and(|w| !w.is_empty()) {
            src = None; // a committed write with no snapshot; the next snapshot is exact again
        }
    }
    src.ok_or_else(|| ApiError::bad("the world after that step can't be restored: its write has no snapshot"))
}

async fn rows(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let seq: i64 = q.get("seq").and_then(|s| s.parse().ok()).ok_or_else(|| ApiError::bad("seq must be an integer"))?;
    let keys: Vec<(String, String)> = q
        .get("keys")
        .map(String::as_str)
        .unwrap_or("")
        .split(',')
        .filter(|k| !k.is_empty())
        .map(|k| match k.split_once('/') {
            Some((t, i)) if world::schema::TABLES.contains(&t) && !i.is_empty() => Ok((t.to_string(), i.to_string())),
            _ => Err(ApiError::bad(format!("bad key {k:?}: expected users/<id>, orders/<id> or products/<id>"))),
        })
        .collect::<Result<_, _>>()?;
    if keys.len() > MAX_KEYS {
        return Err(ApiError::bad(format!("at most {MAX_KEYS} keys per request")));
    }
    if state.store.find_branch(&id).await?.is_none() {
        return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
    }
    let events = state.store.events(&id).await?;
    if !events.iter().any(|e| e.seq == seq) {
        return Err(ApiError::bad("there is no step with that number on this branch"));
    }
    let source = restore_source(&events, seq)?;
    // Create, read and drop run detached, so a client disconnect can't leak the temp database.
    detached(async move {
        let tmp = state
            .worlds
            .new_branch(&format!("rows_{id}_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]), source)
            .await?;
        let read = state
            .worlds
            .with_client(&tmp, async |c| {
                let mut out = serde_json::Map::new();
                for (t, i) in &keys {
                    let row = c.query_opt(&format!("SELECT doc FROM {t} WHERE id = $1"), &[i]).await?;
                    out.insert(format!("{t}/{i}"), row.map(|r| r.get::<_, Value>(0)).unwrap_or(Value::Null));
                }
                Ok(out)
            })
            .await;
        let dropped = state.worlds.drop_db(&tmp).await;
        let out = read?;
        dropped?;
        Ok(Json(json!({"seq": seq, "rows": out})))
    })
    .await
}

// ---------------------------------------------------------------- compare

async fn final_dump(state: &AppState, id: &str) -> anyhow::Result<Value> {
    if let Some(d) = state.dumps.lock().unwrap().get(id) {
        return Ok(d.clone());
    }
    let d = state.worlds.with_client(&format!("branch_{id}"), async |c| world::dump(c).await).await?;
    state.dumps.lock().unwrap().insert(id.to_string(), d.clone());
    Ok(d)
}

async fn finished_branch(state: &AppState, id: &str) -> Result<Branch, ApiError> {
    let b = state.store.find_branch(id).await?.ok_or_else(|| ApiError::not_found(format!("there is no branch \"{id}\"")))?;
    if b.status == BranchStatus::Running {
        return Err(ApiError::conflict(format!("branch \"{id}\" is still running")));
    }
    Ok(b)
}

async fn compare_branches(State(state): State<Arc<AppState>>, Query(q): Query<HashMap<String, String>>) -> ApiResult<Value> {
    let (a, b) = match (q.get("a"), q.get("b")) {
        (Some(a), Some(b)) => (a, b),
        _ => return Err(ApiError::bad("a and b are required")),
    };
    // Existence first (404), then finished (409).
    for id in [a, b] {
        if state.store.find_branch(id).await?.is_none() {
            return Err(ApiError::not_found(format!("there is no branch \"{id}\"")));
        }
    }
    let (ba, bb) = (finished_branch(&state, a).await?, finished_branch(&state, b).await?);
    let (ea, eb) = (state.store.events(a).await?, state.store.events(b).await?);
    let (da, db) = (final_dump(&state, a).await?, final_dump(&state, b).await?);
    Ok(Json(json!({
        "a": compare::summary(&ba, &ea),
        "b": compare::summary(&bb, &eb),
        "first_divergent_seq": compare::first_divergent_seq(&ea, &eb),
        "differing_rows": compare::row_diffs(&da, &db),
    })))
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
            let reserved = ["/api/", "/mcp/", "/gateway/"].iter().any(|p| path.starts_with(p) || path == p.trim_end_matches('/'));
            if reserved || !(method == Method::GET || method == Method::HEAD) {
                return ApiError::not_found("no such route").into_response();
            }
            let rel = path.trim_start_matches('/');
            let safe = FsPath::new(rel).components().all(|c| matches!(c, Component::Normal(_)));
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
        let r = app.clone().oneshot(Request::builder().method(method).uri(uri).body(Body::empty()).unwrap()).await.unwrap();
        let ct = r.headers().get(header::CONTENT_TYPE).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        let st = r.status();
        let body = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (st, ct, String::from_utf8_lossy(&body).to_string())
    }

    #[tokio::test]
    async fn spa_fallback_is_scoped() {
        let root = std::env::temp_dir().join(format!("arena_ui_{}", uuid::Uuid::new_v4().simple()));
        let dist = root.join("dist");
        std::fs::create_dir_all(dist.join("assets")).unwrap();
        std::fs::write(dist.join("index.html"), "INDEX").unwrap();
        std::fs::write(dist.join("assets/a.js"), "JS").unwrap();
        std::fs::write(root.join("secret.txt"), "SECRET").unwrap();
        let app = with_ui(Router::new(), dist);

        assert_eq!(call(&app, "GET", "/").await.2, "INDEX");
        assert_eq!(call(&app, "GET", "/runs/x").await.2, "INDEX");
        let (st, ct, body) = call(&app, "GET", "/assets/a.js").await;
        assert_eq!((st, body.as_str()), (StatusCode::OK, "JS"));
        assert!(ct.starts_with("text/javascript"));
        assert_eq!(call(&app, "GET", "/assets/missing.js").await.0, StatusCode::NOT_FOUND);
        let (st, ct, body) = call(&app, "POST", "/mcp/nope").await;
        assert_eq!(st, StatusCode::NOT_FOUND);
        assert!(ct.starts_with("application/json") && body.contains("error"), "{ct} {body}");
        assert_eq!(call(&app, "GET", "/gateway/x").await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "GET", "/api/x").await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, "POST", "/runs/x").await.0, StatusCode::NOT_FOUND);
        for p in ["/../secret.txt", "/%2e%2e/secret.txt", "/assets/../../secret.txt", "/%2e%2e%2fsecret.txt"] {
            let (_, _, body) = call(&app, "GET", p).await;
            assert!(!body.contains("SECRET"), "{p} escaped");
        }
        let _ = std::fs::remove_dir_all(root);
    }
}
