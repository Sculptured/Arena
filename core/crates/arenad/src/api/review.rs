//! AI review over HTTP. `POST` starts a review (or returns the stored one; `?again=1` makes a new
//! one) and answers at once with the stored row (`status: running` until the model answers);
//! `GET` reads it (404 before any). Off (`GET /api/review/settings`): `POST` is refused (409) and
//! nothing is sent to a model.

use std::{collections::HashMap, sync::Arc};

use anyhow::anyhow;
use axum::{
    extract::{Path, Query, State},
    routing::get,
    Json, Router,
};

use super::{
    compare::{compared_rows, finished_branch, sides, Side},
    pair, ApiError, ApiResult,
};
use crate::{
    app::AppState,
    compare,
    review::{
        self,
        digest::{digest, steps_of, RunRecord, MAX_DIGEST_BYTES},
        output::{Citable, Cites},
        pair::{pair_digest, CompareFacts},
        prompt,
        settings::{settings, ReviewSettings, ReviewerModel},
        suites::{runs_of, suite_digest},
        Job, Shape,
    },
    store::StoredReview,
    suite,
};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/review/settings", get(get_settings))
        .route(
            "/api/branches/{id}/review",
            get(get_branch).post(post_branch),
        )
        .route("/api/compare/review", get(get_compare).post(post_compare))
        .route(
            "/api/suites/compare/review",
            get(get_suites).post(post_suites),
        )
}

type Q = Query<HashMap<String, String>>;

async fn get_settings(State(state): State<Arc<AppState>>) -> Json<ReviewSettings> {
    Json(settings(&state.config.providers).clone())
}

/// The reviewer, or 409 when AI review is off.
fn reviewer(state: &AppState) -> Result<ReviewerModel, ApiError> {
    let s = settings(&state.config.providers);
    match (&s.reviewer, s.enabled) {
        (Some(r), true) => Ok(r.clone()),
        _ => Err(ApiError::conflict(format!(
            "AI review is off: {}",
            s.why.as_deref().unwrap_or("no model")
        ))),
    }
}

async fn stored(state: &AppState, key: &str) -> ApiResult<StoredReview> {
    match state.store.review(key).await? {
        Some(r) => Ok(Json(r)),
        None => Err(ApiError::not_found("there is no review yet")),
    }
}

fn again(q: &HashMap<String, String>) -> bool {
    q.get("again").is_some_and(|v| v == "1" || v == "true")
}

/// The stored review when there is one to keep (done or running, and not asked again), else a new
/// one started: its row, with `make` building the job on a task of its own.
async fn start<F>(
    state: Arc<AppState>,
    key: String,
    shape: Shape,
    again: bool,
    make: F,
) -> ApiResult<StoredReview>
where
    F: std::future::Future<Output = anyhow::Result<Job>> + Send + 'static,
{
    let model = reviewer(&state)?;
    if let Some(r) = state.store.review(&key).await? {
        let keep = r.status == "running" || (r.status == "done" && !again);
        if keep {
            return Ok(Json(r));
        }
    }
    let Some(row) = state
        .store
        .start_review(&key, shape.kind(), &model.label())
        .await?
    else {
        return stored(&state, &key).await;
    };
    tokio::spawn(async move {
        let job = make.await;
        review::run(state, key, model, job).await;
    });
    Ok(Json(row))
}

// ------------------------------------------------------------ one branch

async fn get_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<StoredReview> {
    stored(&state, &review::branch_key(&id)).await
}

async fn post_branch(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Q,
) -> ApiResult<StoredReview> {
    reviewer(&state)?;
    finished_branch(&state, &id).await?;
    let key = review::branch_key(&id);
    let s = state.clone();
    start(state, key, Shape::Run, again(&q), async move {
        branch_job(&s, &id).await
    })
    .await
}

/// The ticket of the run's task, when the benchmark has it.
async fn ticket(state: &AppState, events: &[crate::model::Event]) -> Option<String> {
    let rs = events
        .iter()
        .find(|e| e.kind == crate::model::EventKind::RunStart)?;
    let task = rs.payload["task_id"].as_str()?;
    let bench = state.bench().await.ok()?;
    bench
        .tasks
        .into_iter()
        .find(|t| t.id == task)
        .map(|t| t.ticket)
}

async fn branch_job(state: &AppState, id: &str) -> anyhow::Result<Job> {
    let branch = state.store.branch(id).await?;
    let events = state.store.events(id).await?;
    let ticket = ticket(state, &events).await;
    let rec = RunRecord {
        branch: &branch,
        events: &events,
        ticket: ticket.as_deref(),
    };
    let d = digest(&rec, MAX_DIGEST_BYTES);
    let citable = Citable {
        steps_a: steps_of(&events),
        agents: branch.team.agents.iter().map(|a| a.id.clone()).collect(),
        ..Citable::default()
    };
    Ok(Job {
        prompt: prompt::run_prompt(&d),
        citable,
        shape: Shape::Run,
    })
}

// ------------------------------------------------------------ two branches

async fn get_compare(State(state): State<Arc<AppState>>, Query(q): Q) -> ApiResult<StoredReview> {
    let (a, b) = pair(&q)?;
    stored(&state, &review::compare_key(&a, &b)).await
}

async fn post_compare(State(state): State<Arc<AppState>>, Query(q): Q) -> ApiResult<StoredReview> {
    reviewer(&state)?;
    let (a, b) = pair(&q)?;
    for id in [&a, &b] {
        finished_branch(&state, id).await?;
    }
    let key = review::compare_key(&a, &b);
    let s = state.clone();
    start(
        state,
        key,
        Shape::Compare(Cites::Steps),
        again(&q),
        async move { compare_job(&s, &a, &b).await },
    )
    .await
}

/// `ApiError` as an error a job can carry.
fn job_err(e: ApiError) -> anyhow::Error {
    anyhow!(e.1)
}

/// What `GET /api/compare` finds for the pair, as the reviewer reads it.
struct Compared {
    first_divergent_seq: Option<i64>,
    rows: Vec<serde_json::Value>,
    skipped: Vec<String>,
}

async fn compared(state: &AppState, a: &Side, b: &Side) -> anyhow::Result<Compared> {
    let skipped = compare::not_compared(&state.hub, &a.end, &b.end).await?;
    let rows = (compared_rows(state, &a.end, &b.end, &skipped).await).map_err(job_err)?;
    Ok(Compared {
        first_divergent_seq: compare::first_divergent_seq(&a.events, &b.events),
        rows,
        skipped,
    })
}

fn record<'a>(s: &'a Side, ticket: Option<&'a str>) -> RunRecord<'a> {
    RunRecord {
        branch: &s.branch,
        events: &s.events,
        ticket,
    }
}

async fn compare_job(state: &AppState, a: &str, b: &str) -> anyhow::Result<Job> {
    let (ia, ib) = (a.to_string(), b.to_string());
    let [sa, sb] = sides(state, [&ia, &ib]).await.map_err(job_err)?;
    let c = compared(state, &sa, &sb).await?;
    let facts = CompareFacts {
        first_divergent_seq: c.first_divergent_seq,
        differing_rows: &c.rows,
        not_compared: &c.skipped,
    };
    let ticket = ticket(state, &sa.events).await;
    let (ra, rb) = (
        record(&sa, ticket.as_deref()),
        record(&sb, ticket.as_deref()),
    );
    let agents = sa.branch.team.agents.iter().chain(&sb.branch.team.agents);
    let citable = Citable {
        steps_a: steps_of(&sa.events),
        steps_b: steps_of(&sb.events),
        agents: agents.map(|x| x.id.clone()).collect(),
        ..Citable::default()
    };
    let prompt = prompt::compare_prompt(&pair_digest(&ra, &rb, &facts));
    Ok(Job {
        prompt,
        citable,
        shape: Shape::Compare(Cites::Steps),
    })
}

// ------------------------------------------------------------ two suites

async fn get_suites(State(state): State<Arc<AppState>>, Query(q): Q) -> ApiResult<StoredReview> {
    let (a, b) = pair(&q)?;
    stored(&state, &review::suites_key(&a, &b)).await
}

async fn find_suite(state: &AppState, id: &str) -> Result<suite::Suite, ApiError> {
    state
        .store
        .suite(id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("there is no suite \"{id}\"")))
}

async fn post_suites(State(state): State<Arc<AppState>>, Query(q): Q) -> ApiResult<StoredReview> {
    reviewer(&state)?;
    let (a, b) = pair(&q)?;
    let (sa, sb) = (find_suite(&state, &a).await?, find_suite(&state, &b).await?);
    let key = review::suites_key(&a, &b);
    let s = state.clone();
    start(
        state,
        key,
        Shape::Compare(Cites::Runs),
        again(&q),
        async move { suites_job(&s, sa, sb).await },
    )
    .await
}

async fn suites_job(state: &AppState, a: suite::Suite, b: suite::Suite) -> anyhow::Result<Job> {
    let ra = suite::load(&state.store, a).await?;
    let rb = suite::load(&state.store, b).await?;
    let c = suite::compare(&ra, &rb);
    let d = suite_digest(&ra, &rb, &c);
    let agents = ra.suite.team.agents.iter().chain(&rb.suite.team.agents);
    let citable = Citable {
        runs_a: runs_of(&ra),
        runs_b: runs_of(&rb),
        agents: agents.map(|x| x.id.clone()).collect(),
        ..Citable::default()
    };
    Ok(Job {
        prompt: prompt::suites_prompt(&d),
        citable,
        shape: Shape::Compare(Cites::Runs),
    })
}
