//! What a branch's events say about its world: the run's benchmark, seed, world, fingerprints
//! and base saves (from `RunStart`), per part the save holding its state at a step, the arena
//! clock at a step, its final saves, and every save the events name. Runs recorded before integrations have a `snapshot` per write and no
//! `saves`: they read as part `retail`, with `world_base` as the base and the database
//! `branch_<id>` as the final state.

use std::collections::{BTreeMap, BTreeSet};

use parts::proto::WorldPart;
use serde_json::{Map, Value};

use crate::{
    model::{Event, EventKind, Rules},
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
    /// The parts of the run's world (empty for a run recorded before integrations).
    pub world: Vec<WorldPart>,
    /// Each part's integration fingerprint at the run's start (empty when not recorded).
    pub fingerprints: Map<String, Value>,
    /// False when the branch has no `RunStart` (its setup failed before recording one): it names
    /// no saves and has no world.
    pub started: bool,
    /// True for a run recorded before integrations (its `RunStart` names no saves).
    pub legacy: bool,
}

fn legacy(save: &str) -> Saves {
    Saves::from([(LEGACY_PART.to_string(), save.to_string())])
}

/// A `{part: save}` object, None when `v` is not one (or names no save).
fn saves_in(v: &Value) -> Option<Saves> {
    serde_json::from_value::<Saves>(v.clone())
        .ok()
        .filter(|s| !s.is_empty())
}

/// The run facts of a branch's events (a run recorded before integrations: retail, seed 0,
/// base `world_base`; a branch without `RunStart`: no base). A run whose parts all keep no saves
/// records `saves: {}`.
pub fn run_info(events: &[Event]) -> RunInfo {
    let start = events
        .iter()
        .find(|e| e.kind == EventKind::RunStart)
        .map(|e| &e.payload);
    let base = start.and_then(|p| serde_json::from_value::<Saves>(p["saves"].clone()).ok());
    let world = start.and_then(|p| serde_json::from_value(p["world"].clone()).ok());
    let fingerprints = start.and_then(|p| p["fingerprints"].as_object().cloned());
    RunInfo {
        benchmark: start
            .and_then(|p| p["benchmark"].as_str())
            .unwrap_or(LEGACY_PART)
            .to_string(),
        seed: start.and_then(|p| p["seed"].as_u64()).unwrap_or(0),
        started: start.is_some(),
        legacy: start.is_some() && base.is_none(),
        base: match (start, base) {
            (_, Some(b)) => b,
            (Some(_), None) => legacy(LEGACY_BASE),
            (None, None) => Saves::new(),
        },
        world: world.unwrap_or_default(),
        fingerprints: fingerprints.unwrap_or_default(),
    }
}

/// The rules the run of `events` runs under, from its `RunStart` (legacy when there is none).
pub fn rules(events: &[Event]) -> Rules {
    let start = events.iter().find(|e| e.kind == EventKind::RunStart);
    start.map_or_else(Rules::legacy, |e| Rules::of_run_start(&e.payload))
}

/// The arena clock just before step `before_seq`: the `clock_ms` of the last event before it
/// that has one (a tool call or world event), else 0.
pub fn clock_at(events: &[Event], before_seq: i64) -> i64 {
    events
        .iter()
        .rev()
        .filter(|e| e.seq < before_seq)
        .find_map(|e| e.payload["clock_ms"].as_i64())
        .unwrap_or(0)
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
    saves_at_except(events, seq, base, &BTreeSet::new())
}

/// `saves_at` for every part but `skip` (parts restored without a save, by replay).
pub fn saves_at_except(
    events: &[Event],
    seq: i64,
    base: &Saves,
    skip: &BTreeSet<String>,
) -> Result<Saves, NoSave> {
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
        .filter(|(part, _)| !skip.contains(part))
        .map(|(part, s)| {
            s.map(|s| (part.clone(), s))
                .map_err(|seq| NoSave { part, seq })
        })
        .collect()
}

/// Per part, the save holding the branch's final state: its check's `final_saves` (empty when
/// no part keeps saves), else for a run recorded before integrations its database
/// `branch_<id>`. None while it has neither.
pub fn final_saves(branch: &str, events: &[Event]) -> Option<Saves> {
    let checked = events
        .iter()
        .rev()
        .find(|e| e.kind == EventKind::Check)
        .and_then(|e| serde_json::from_value::<Saves>(e.payload["final_saves"].clone()).ok());
    checked.or_else(|| {
        run_info(events)
            .legacy
            .then(|| legacy(&format!("branch_{branch}")))
    })
}

/// A save named by a branch's events, and the part it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct NamedSave {
    pub part: String,
    pub save: String,
}

/// Every save `branch`'s events name: the run's start saves, each tool call's saves (or legacy
/// snapshot) and the final saves.
pub fn named_saves(branch: &str, events: &[Event]) -> BTreeSet<NamedSave> {
    let recorded = events.iter().filter_map(|e| match e.kind {
        EventKind::RunStart => saves_in(&e.payload["saves"]),
        EventKind::ToolCall => saves_of(&e.payload),
        _ => None,
    });
    recorded
        .chain(final_saves(branch, events))
        .flatten()
        .map(|(part, save)| NamedSave { part, save })
        .collect()
}

/// True for a tool call that committed a write: one that names keys it wrote, refused or not (a
/// part may change on a call it refuses, when a scheduled disruption fires).
/// True when the call changed its part: the world's keys or the part's internal ones (either
/// way the part must have been saved with the step).
fn wrote(payload: &Value) -> bool {
    let any = |k: &str| payload[k].as_array().is_some_and(|w| !w.is_empty());
    any("writes") || any("internal")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, kind: EventKind, payload: Value) -> Event {
        crate::testkit::event(seq, None, kind, payload)
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
                base: legacy("world_base"),
                world: vec![],
                fingerprints: Map::new(),
                started: true,
                legacy: true,
            }
        );
        assert_eq!(saves_at(&evs, 1, &info.base).unwrap(), legacy("world_base"));
        assert_eq!(saves_at(&evs, 2, &info.base).unwrap(), legacy("snap_a"));
    }

    fn names(evs: &[Event], branch: &str) -> Vec<String> {
        named_saves(branch, evs)
            .into_iter()
            .map(|n| n.save)
            .collect()
    }

    #[test]
    fn final_and_named_saves() {
        let evs = vec![
            ev(
                1,
                EventKind::RunStart,
                json!({"benchmark": "b", "saves": {"retail": "base_r"}}),
            ),
            write(2, "retail", json!({"retail": "s_r1"})),
            ev(
                3,
                EventKind::Check,
                json!({"passed": true, "final_saves": {"retail": "s_r2"}}),
            ),
        ];
        assert_eq!(final_saves("x", &evs), Some(legacy("s_r2")));
        assert_eq!(final_saves("x", &evs[..2]), None, "no check yet");
        assert_eq!(names(&evs, "x"), ["base_r", "s_r1", "s_r2"]);
    }

    #[test]
    fn a_run_whose_parts_keep_no_saves_is_not_legacy() {
        let evs = vec![
            ev(
                1,
                EventKind::RunStart,
                json!({"benchmark": "b", "seed": 3, "saves": {}, "fingerprints": {"c": {"v": 1}}}),
            ),
            write(2, "c", Value::Null),
        ];
        let info = run_info(&evs);
        assert!(!info.legacy && info.base.is_empty());
        assert_eq!(info.fingerprints["c"], json!({"v": 1}));
        // A replayed part's writes have no save, and need none.
        let skip = BTreeSet::from(["c".to_string()]);
        assert_eq!(
            saves_at_except(&evs, 2, &info.base, &skip),
            Ok(Saves::new())
        );
        assert_eq!(saves_at(&evs, 2, &info.base).unwrap_err().part, "c");
    }

    #[test]
    fn the_clock_at_a_step_is_the_last_one_recorded_before_it() {
        let call = |seq, clock: Value| ev(seq, EventKind::ToolCall, json!({"clock_ms": clock}));
        let evs = vec![
            ev(1, EventKind::RunStart, json!({})),
            call(2, json!(120)),
            ev(3, EventKind::WorldEvent, json!({"clock_ms": 450})),
            ev(4, EventKind::ModelCall, json!({})),
            call(5, Value::Null),
        ];
        assert_eq!(clock_at(&evs, 2), 0);
        assert_eq!(clock_at(&evs, 3), 120);
        assert_eq!(clock_at(&evs, 5), 450);
        assert_eq!(clock_at(&evs, 99), 450);
    }

    #[test]
    fn a_branch_without_run_start_names_nothing() {
        let info = run_info(&[]);
        assert!(!info.started && !info.legacy && info.base.is_empty());
        assert_eq!(final_saves("b1", &[]), None);
        assert!(named_saves("b1", &[]).is_empty());
    }

    #[test]
    fn legacy_final_state_is_the_branch_database() {
        // The base `world_base` is not named: it is no save of the run.
        let old = vec![
            ev(1, EventKind::RunStart, json!({"task_id": "27"})),
            ev(
                2,
                EventKind::ToolCall,
                json!({"name": "w", "error": null, "writes": ["orders/#W1"], "snapshot": "snap_a"}),
            ),
            ev(
                3,
                EventKind::Check,
                json!({"passed": true, "gold_db": "gold_27"}),
            ),
        ];
        assert_eq!(final_saves("b1", &old), Some(legacy("branch_b1")));
        assert_eq!(names(&old, "b1"), ["branch_b1", "snap_a"]);
        assert_eq!(named_saves("b1", &old).first().unwrap().part, "retail");
    }
}
