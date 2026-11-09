//! Events a sandbox around an agent reports while the branch runs, such as an OpenShell policy denial
//! (`deploy/openshell`): `POST /api/branches/{id}/sandbox-events` records each as a world event of the
//! part `sandbox`, at the branch's clock, with the agent it was about in its data.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    routing::post,
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::{parse_body, validate_agent_id, ApiError, ApiResult};
use crate::{app::AppState, model::*};

pub(super) fn router() -> Router<Arc<AppState>> {
    Router::new().route("/api/branches/{id}/sandbox-events", post(report))
}

/// The world events' part: what reported them is the sandbox, not a part of the world.
pub const PART: &str = "sandbox";

/// The most events one report may carry.
const MAX_EVENTS: usize = 200;

#[derive(Deserialize)]
struct Report {
    agent: String,
    events: Vec<SandboxEvent>,
}

#[derive(Deserialize)]
struct SandboxEvent {
    kind: String,
    text: String,
    #[serde(default)]
    data: Value,
}

/// The world event a sandbox's report of `e` about `agent` is recorded as.
fn payload(agent: &str, e: SandboxEvent, clock_ms: i64) -> WorldEventPayload {
    WorldEventPayload {
        part: PART.into(),
        kind: e.kind,
        text: e.text,
        data: json!({"agent": agent, "sandbox": e.data}),
        clock_ms,
    }
}

fn check(r: &Report) -> Result<(), ApiError> {
    validate_agent_id(&r.agent)?;
    if r.events.len() > MAX_EVENTS {
        return Err(ApiError::bad(format!(
            "at most {MAX_EVENTS} events per report"
        )));
    }
    if r.events
        .iter()
        .any(|e| e.kind.trim().is_empty() || e.text.trim().is_empty())
    {
        return Err(ApiError::bad("each event needs a kind and a text"));
    }
    Ok(())
}

/// Records the report's events on the live branch `id`; 404 when it isn't live, 400 for an agent
/// it doesn't have.
async fn report(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<Value> {
    let r: Report = parse_body(&body)?;
    check(&r)?;
    let ctx = state.live.read().unwrap().get(&id).cloned();
    let ctx = ctx.ok_or_else(|| ApiError::not_found(format!("no live branch \"{id}\"")))?;
    if ctx.agent(&r.agent).is_none() {
        return Err(ApiError::bad(format!(
            "the branch has no agent \"{}\"",
            r.agent
        )));
    }
    let n = r.events.len();
    for e in r.events {
        let p = serde_json::to_value(payload(&r.agent, e, ctx.clock.now_ms()))
            .map_err(anyhow::Error::from)?;
        ctx.append(&state.store, None, EventKind::WorldEvent, p)
            .await?;
    }
    Ok(Json(json!({"recorded": n})))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(events: Value) -> Report {
        serde_json::from_value(json!({"agent": "planner", "events": events})).unwrap()
    }

    #[test]
    fn a_denial_becomes_a_world_event_of_the_sandbox_naming_the_agent() {
        let mut r = report(json!([{"kind": "denial", "text": "example.com:443 denied",
                                   "data": {"source": "openshell"}}]));
        let p = payload("planner", r.events.remove(0), 1200);
        assert_eq!(
            (p.part.as_str(), p.kind.as_str(), p.clock_ms),
            ("sandbox", "denial", 1200)
        );
        assert_eq!(
            p.data,
            json!({"agent": "planner", "sandbox": {"source": "openshell"}})
        );
    }

    #[test]
    fn a_report_needs_kinds_texts_and_a_valid_agent() {
        assert!(check(&report(json!([{"kind": "denial", "text": "x"}]))).is_ok());
        assert!(check(&report(json!([{"kind": " ", "text": "x"}]))).is_err());
        let bad_agent: Report =
            serde_json::from_value(json!({"agent": "a b", "events": []})).unwrap();
        assert!(check(&bad_agent).is_err());
        let many: Vec<Value> = (0..=MAX_EVENTS)
            .map(|_| json!({"kind": "k", "text": "t"}))
            .collect();
        assert!(check(&report(Value::Array(many))).is_err());
    }
}
