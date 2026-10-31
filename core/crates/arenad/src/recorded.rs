//! What a branch's events say about its world: the run's benchmark, seed and base saves (from
//! `RunStart`), and per part the save holding its state at a step. Runs recorded before
//! integrations have a `snapshot` per write and no `saves`: they read as part `retail`, with
//! `world_base` as the base.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    model::{Event, EventKind},
    world::Saves,
};

/// The part of runs recorded before integrations.
pub const LEGACY_PART: &str = "retail";
/// The starting database of runs recorded before integrations.
pub const LEGACY_BASE: &str = "world_base";

/// The run facts `RunStart` records.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInfo {
    pub benchmark: String,
    pub seed: u64,
    /// The saves the run's worlds start from.
    pub base: Saves,
}

fn legacy(save: &str) -> Saves {
    Saves::from([(LEGACY_PART.to_string(), save.to_string())])
}

/// The run facts of a branch's events (a run recorded before integrations: retail, seed 0,
/// base `world_base`).
pub fn run_info(events: &[Event]) -> RunInfo {
    let start = events
        .iter()
        .find(|e| e.kind == EventKind::RunStart)
        .map(|e| &e.payload);
    let base = start
        .and_then(|p| serde_json::from_value::<Saves>(p["saves"].clone()).ok())
        .filter(|s| !s.is_empty());
    RunInfo {
        benchmark: start
            .and_then(|p| p["benchmark"].as_str())
            .unwrap_or(LEGACY_PART)
            .to_string(),
        seed: start.and_then(|p| p["seed"].as_u64()).unwrap_or(0),
        base: base.unwrap_or_else(|| legacy(LEGACY_BASE)),
    }
}

/// The saves a tool call made: `saves`, or a legacy `snapshot` read as `{"retail": snapshot}`.
pub fn saves_of(payload: &Value) -> Option<Saves> {
    match serde_json::from_value::<Saves>(payload["saves"].clone()) {
        Ok(s) => Some(s),
        Err(_) => payload["snapshot"].as_str().map(legacy),
    }
}

/// A part whose state at the step can't be restored: its committed write at `seq` has no save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoSave {
    pub part: String,
    pub seq: i64,
}

/// Per part, the save holding its state just after step `seq`: its last save at or before it,
/// else `base`. A committed write to a part with no save makes that part unrestorable until its
/// next save (the earliest such write is reported).
pub fn saves_at(events: &[Event], seq: i64, base: &Saves) -> Result<Saves, NoSave> {
    let mut at: BTreeMap<String, Result<String, i64>> = base
        .iter()
        .map(|(p, s)| (p.clone(), Ok(s.clone())))
        .collect();
    for e in events
        .iter()
        .filter(|e| e.seq <= seq && e.kind == EventKind::ToolCall)
    {
        let saved = saves_of(&e.payload).unwrap_or_default();
        let part = e.payload["part"].as_str().unwrap_or(LEGACY_PART);
        if wrote(&e.payload) && !saved.contains_key(part) {
            at.insert(part.to_string(), Err(e.seq));
        }
        at.extend(saved.into_iter().map(|(p, s)| (p, Ok(s))));
    }
    at.into_iter()
        .map(|(part, s)| {
            s.map(|s| (part.clone(), s))
                .map_err(|seq| NoSave { part, seq })
        })
        .collect()
}

/// True for a tool call that committed a write.
fn wrote(payload: &Value) -> bool {
    payload["error"].is_null() && payload["writes"].as_array().is_some_and(|w| !w.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: chrono::Utc::now(),
            agent: None,
            kind,
            payload,
        }
    }

    fn write(seq: i64, part: &str, saves: Value) -> Event {
        ev(
            seq,
            EventKind::ToolCall,
            json!({"name": "w", "error": null, "writes": ["k/1"], "part": part, "saves": saves}),
        )
    }

    fn m(r: &str, p: &str) -> Saves {
        Saves::from([
            ("plant".to_string(), p.to_string()),
            ("retail".to_string(), r.to_string()),
        ])
    }

    #[test]
    fn saves_are_per_part() {
        let evs = vec![
            ev(
                1,
                EventKind::RunStart,
                json!({"benchmark": "b", "seed": 7, "saves": {"retail": "base_r", "plant": "base_p"}}),
            ),
            write(2, "retail", json!({"retail": "s_r1"})),
            write(3, "plant", json!({"plant": "s_p1"})),
            write(4, "plant", Value::Null),
            write(5, "plant", json!({"plant": "s_p2"})),
        ];
        let info = run_info(&evs);
        assert_eq!((info.benchmark.as_str(), info.seed), ("b", 7));
        let at = |seq| saves_at(&evs, seq, &info.base);
        assert_eq!(at(1).unwrap(), m("base_r", "base_p"));
        assert_eq!(at(2).unwrap(), m("s_r1", "base_p"));
        assert_eq!(at(3).unwrap(), m("s_r1", "s_p1"));
        assert_eq!(
            at(4).unwrap_err(),
            NoSave {
                part: "plant".into(),
                seq: 4
            }
        );
        // A later save is a full copy: exact again past the unsaved write.
        assert_eq!(at(5).unwrap(), m("s_r1", "s_p2"));
    }

    #[test]
    fn legacy_runs_read_as_retail() {
        let evs = vec![
            ev(1, EventKind::RunStart, json!({"task_id": "27"})),
            ev(
                2,
                EventKind::ToolCall,
                json!({"name": "w", "error": null, "writes": ["orders/#W1"], "snapshot": "snap_a"}),
            ),
        ];
        let info = run_info(&evs);
        assert_eq!(
            info,
            RunInfo {
                benchmark: "retail".into(),
                seed: 0,
                base: legacy("world_base")
            }
        );
        assert_eq!(saves_at(&evs, 1, &info.base).unwrap(), legacy("world_base"));
        assert_eq!(saves_at(&evs, 2, &info.base).unwrap(), legacy("snap_a"));
    }
}
