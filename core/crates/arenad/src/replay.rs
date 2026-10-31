//! Replay: restoring a part by repeating its recorded inputs. A part's inputs are its own tool
//! calls (the tools it lists, recorded with its name as `part`) before the fork point, in order,
//! each at the clock it ran at. The replayed part starts fresh with the run's seed at clock 0;
//! for each input it is advanced to the input's clock, and the call must give back what it gave
//! the first time.

use serde_json::Value;

use crate::{
    exec::outcome_of,
    model::{Event, EventKind},
    world::{World, WorldError},
};

/// The part's error code for a fingerprint mismatch.
pub const FINGERPRINT_MISMATCH: i64 = -32003;

/// One recorded input of a part.
#[derive(Debug, Clone, PartialEq)]
pub struct Input {
    pub seq: i64,
    pub name: String,
    pub args: Value,
    pub clock_ms: i64,
    /// What the call gave back: its result, or its error.
    pub outcome: Result<Value, String>,
}

/// Why a replay failed.
#[derive(Debug)]
pub enum ReplayError {
    /// The input at `seq` gave back something else than the first time.
    Diverged {
        seq: i64,
    },
    /// The part refused: it runs on a different machine or version.
    Fingerprint,
    Failed(WorldError),
}

impl From<WorldError> for ReplayError {
    fn from(e: WorldError) -> ReplayError {
        match e.code {
            Some(FINGERPRINT_MISMATCH) => ReplayError::Fingerprint,
            _ => ReplayError::Failed(e),
        }
    }
}

/// True for a recorded call of `part`'s own tools (one of `tools`).
fn is_input(e: &Event, part: &str, tools: &[String]) -> bool {
    e.kind == EventKind::ToolCall
        && e.payload["part"].as_str() == Some(part)
        && e.payload["name"]
            .as_str()
            .is_some_and(|n| tools.iter().any(|t| t == n))
}

fn recorded_outcome(payload: &Value) -> Result<Value, String> {
    match payload["error"].as_str() {
        Some(e) => Err(e.to_string()),
        None => Ok(payload["result"].clone()),
    }
}

/// `part`'s inputs before `before_seq`, in order.
pub fn inputs(events: &[Event], part: &str, tools: &[String], before_seq: i64) -> Vec<Input> {
    events
        .iter()
        .filter(|e| e.seq < before_seq && is_input(e, part, tools))
        .map(|e| Input {
            seq: e.seq,
            name: e.payload["name"].as_str().unwrap_or_default().to_string(),
            args: e.payload["args"].clone(),
            clock_ms: e.payload["clock_ms"].as_i64().unwrap_or(0),
            outcome: recorded_outcome(&e.payload),
        })
        .collect()
}

/// A rough estimate of how long replaying `part` up to `before_seq` takes: per input, the time
/// since the event before it, summed.
pub fn estimate_seconds(events: &[Event], part: &str, tools: &[String], before_seq: i64) -> f64 {
    let ms: i64 = events
        .windows(2)
        .filter(|w| w[1].seq < before_seq && is_input(&w[1], part, tools))
        .map(|w| (w[1].at - w[0].at).num_milliseconds().max(0))
        .sum();
    ms as f64 / 1000.0
}

/// Replays `inputs` on `part` of a world that started it fresh, then advances it to `until_ms`
/// (the fork point's clock).
pub async fn replay(
    world: &World,
    part: &str,
    inputs: &[Input],
    until_ms: i64,
) -> Result<(), ReplayError> {
    for i in inputs {
        world.advance_part(part, i.clock_ms).await?;
        let out = world
            .call_part_tool(part, &i.name, i.args.clone(), i.clock_ms)
            .await?;
        if outcome_of(&out) != i.outcome {
            return Err(ReplayError::Diverged { seq: i.seq });
        }
    }
    world.advance_part(part, until_ms).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use serde_json::json;

    fn ev(seq: i64, ms: i64, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: Utc.timestamp_opt(1_000_000, 0).unwrap() + Duration::milliseconds(ms),
            agent: None,
            kind,
            payload,
        }
    }

    fn tool(seq: i64, ms: i64, part: &str, name: &str, error: Option<&str>) -> Event {
        let p = json!({"name": name, "args": {"n": seq}, "result": if error.is_some() { Value::Null } else { json!({"v": seq}) },
                       "error": error, "part": part, "clock_ms": seq * 100});
        ev(seq, ms, EventKind::ToolCall, p)
    }

    fn events() -> Vec<Event> {
        vec![
            ev(1, 0, EventKind::RunStart, json!({})),
            ev(2, 100, EventKind::ModelCall, json!({})),
            tool(3, 400, "c", "add", None),
            tool(4, 500, "db", "add", None),
            tool(5, 1500, "c", "get", Some("nope")),
            tool(6, 1600, "c", "ring", None),
            tool(7, 2600, "c", "add", None),
        ]
    }

    fn tools() -> Vec<String> {
        vec!["add".into(), "get".into()]
    }

    #[test]
    fn inputs_are_the_parts_own_calls_before_the_fork_point() {
        let got = inputs(&events(), "c", &tools(), 7);
        let seqs: Vec<i64> = got.iter().map(|i| i.seq).collect();
        assert_eq!(seqs, [3, 5]);
        assert_eq!(
            got[0],
            Input {
                seq: 3,
                name: "add".into(),
                args: json!({"n": 3}),
                clock_ms: 300,
                outcome: Ok(json!({"v": 3})),
            }
        );
        assert_eq!(got[1].outcome, Err("nope".to_string()));
        assert_eq!(inputs(&events(), "c", &tools(), 99).len(), 3);
    }

    #[test]
    fn the_estimate_sums_the_time_before_each_input() {
        // 3: 300 ms after the model call; 5: 1000 ms after seq 4.
        assert_eq!(estimate_seconds(&events(), "c", &tools(), 7), 1.3);
        assert_eq!(estimate_seconds(&events(), "db", &tools(), 7), 0.1);
        assert_eq!(estimate_seconds(&events(), "c", &tools(), 3), 0.0);
    }

    #[test]
    fn a_fingerprint_refusal_is_its_own_error() {
        let e = WorldError {
            message: "x".into(),
            fatal: false,
            code: Some(-32003),
        };
        assert!(matches!(ReplayError::from(e), ReplayError::Fingerprint));
        let e = WorldError {
            message: "x".into(),
            fatal: false,
            code: Some(-32001),
        };
        assert!(matches!(ReplayError::from(e), ReplayError::Failed(_)));
    }
}
