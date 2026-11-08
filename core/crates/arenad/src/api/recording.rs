//! A branch's plant recording, for the world view's video panel: an MP4 a recording part wrote
//! (Isaac Sim's frames fall on arena time, so the video's second t is the branch's arena second t)
//! and its JSON sidecar, found in `ARENA_RECORDINGS` by the branch id in the file's name (the
//! plant part names it `<stem>-b_<branch>_<part>.mp4`). A fork without one of its own shows its
//! parent's. `GET /api/branches/{id}/recording` says what there is; `.../recording.mp4` serves the
//! video, with byte ranges so a player can seek.

use std::{path::PathBuf, sync::Arc};

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::{json, Value};

use super::{ApiError, ApiResult};
use crate::app::AppState;

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/branches/{id}/recording", get(recording))
        .route("/api/branches/{id}/recording.mp4", get(video))
}

/// The folder recordings are fetched into, if one is set.
fn folder() -> Option<PathBuf> {
    std::env::var_os("ARENA_RECORDINGS").map(PathBuf::from)
}

/// The MP4 in `dir` whose name has `branch` in it (the newest when several do).
fn mp4_of(dir: &std::path::Path, branch: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mp4"))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains(branch))
        })
        .collect();
    found.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    found.pop()
}

/// The recording of `id`, or of its nearest ancestor that has one, with the branch it is of.
async fn find(state: &AppState, id: &str) -> Result<Option<(String, PathBuf)>, ApiError> {
    let Some(dir) = folder() else { return Ok(None) };
    let mut at = Some(id.to_string());
    while let Some(branch) = at {
        let Some(b) = state.store.find_branch(&branch).await? else {
            return Err(ApiError::not_found(format!(
                "there is no branch \"{branch}\""
            )));
        };
        if let Some(path) = mp4_of(&dir, &branch) {
            return Ok(Some((branch, path)));
        }
        at = b.parent_id;
    }
    Ok(None)
}

/// `{video: url | null, of: branch, sidecar: {...} | null}`.
async fn recording(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<Value> {
    let Some((of, path)) = find(&state, &id).await? else {
        return Ok(Json(json!({"video": null, "of": null, "sidecar": null})));
    };
    let sidecar = std::fs::read_to_string(path.with_extension("json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let url = format!("/api/branches/{id}/recording.mp4");
    Ok(Json(json!({"video": url, "of": of, "sidecar": sidecar})))
}

/// `bytes=a-b` (b optional) of a file of `len` bytes, as an inclusive range.
fn range(headers: &HeaderMap, len: u64) -> Option<(u64, u64)> {
    let spec = headers
        .get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    let end = b
        .trim()
        .parse::<u64>()
        .ok()
        .unwrap_or(len.saturating_sub(1));
    (start <= end && end < len).then_some((start, end))
}

async fn video(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let Some((_, path)) = find(&state, &id).await? else {
        return Err(ApiError::not_found(format!(
            "branch \"{id}\" has no recording"
        )));
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;
    let len = bytes.len() as u64;
    let mp4 = (header::CONTENT_TYPE, "video/mp4");
    match range(&headers, len) {
        Some((a, b)) => {
            let body = bytes[a as usize..=b as usize].to_vec();
            let span = (header::CONTENT_RANGE, format!("bytes {a}-{b}/{len}"));
            Ok((
                StatusCode::PARTIAL_CONTENT,
                [mp4, (header::ACCEPT_RANGES, "bytes")],
                [span],
                body,
            )
                .into_response())
        }
        None => Ok(([mp4, (header::ACCEPT_RANGES, "bytes")], bytes).into_response()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_is_read_inclusively_and_checked() {
        let mut h = HeaderMap::new();
        assert_eq!(range(&h, 10), None);
        h.insert(header::RANGE, "bytes=2-5".parse().unwrap());
        assert_eq!(range(&h, 10), Some((2, 5)));
        h.insert(header::RANGE, "bytes=4-".parse().unwrap());
        assert_eq!(range(&h, 10), Some((4, 9)));
        h.insert(header::RANGE, "bytes=9-20".parse().unwrap());
        assert_eq!(range(&h, 10), None);
    }

    #[test]
    fn the_branchs_mp4_is_found_by_its_id() {
        let dir = std::env::temp_dir().join(format!("arena-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("hero-b_abc123_plant.mp4"), b"x").unwrap();
        std::fs::write(dir.join("hero-b_abc123_plant.json"), b"{}").unwrap();
        let found = mp4_of(&dir, "abc123");
        let other = mp4_of(&dir, "zzz");
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            found.unwrap().file_name().unwrap(),
            "hero-b_abc123_plant.mp4"
        );
        assert!(other.is_none());
    }
}
