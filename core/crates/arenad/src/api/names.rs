//! Display names are optional metadata: task ids and event records stay unchanged.
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{parse_body, ApiError, ApiResult};
use crate::app::AppState;

#[derive(Deserialize)]
struct Name {
    name: String,
}

pub(super) async fn set(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Value> {
    let req: Name = parse_body(&body)?;
    let name = req.name.trim();
    if name.is_empty() || name.len() > 300 {
        return Err(ApiError::bad("name must be 1 to 300 bytes"));
    }
    if !state.store.name_run(&id, name).await? {
        return Err(ApiError::not_found(format!("no run {id}")));
    }
    Ok(Json(json!({"id": id, "name": name})))
}
