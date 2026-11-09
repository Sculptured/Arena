//! The docs of some keys of a part as they were just after a step (`GET /api/branches/{id}/rows`).

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::{Path, Query, State},
    Json,
};
use parts::proto::WorldPart;
use serde_json::{json, Value};

use super::{branch_or_404, ApiError, ApiResult};
use crate::{
    app::AppState,
    model::*,
    recorded::{run_info, saves_at},
    saves::{self, part_named, part_refusal, world_of},
};

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

pub(super) async fn rows(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
) -> ApiResult<Value> {
    let q = parse_rows_query(&q)?;
    branch_or_404(&state, &id).await?;
    let events = state.store.events(&id).await?;
    Ok(Json(read_rows(&state, &events, &q).await?))
}
