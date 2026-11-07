//! Branch comparison: per-branch totals, first divergent step, differing final rows.

use anyhow::{anyhow, Result};
use parts::{
    proto::{is_internal_key, WorldPart},
    Hub,
};
use serde_json::{json, Map, Value};

use crate::{
    metrics::metrics,
    model::*,
    prices,
    restore::keeps_saves,
    saves::{self, part_code},
    world::Saves,
};

/// `{status, restore_mode, restored, seconds, tokens, cost_usd, cost_source}` for a finished
/// branch (`restore_mode` null and `restored` empty for a run's first branch). Only the branch's
/// own events count (`seq >= start_seq`): the inherited prefix belongs to its parent.
pub fn summary(branch: &Branch, events: &[Event]) -> Value {
    let from = branch.start_seq.unwrap_or(0);
    let ends = events
        .iter()
        .filter(|e| e.seq >= from && e.kind == EventKind::AgentEnd)
        .filter_map(|e| serde_json::from_value::<AgentEndPayload>(e.payload.clone()).ok())
        .collect::<Vec<_>>();
    let seconds = branch
        .ended_at
        .map(|t| (t - branch.created_at).num_milliseconds() as f64 / 1000.0);
    // Tokens as arena's metrics count them (a client's calls once each), not the ends' own sums.
    let m = metrics(branch, events, prices::table(), chrono::Utc::now());
    json!({
        "status": branch.status,
        "restore_mode": branch.restore_mode,
        "restored": branch.restored,
        "seconds": seconds,
        "tokens": m.tokens,
        "cost_usd": ends.iter().try_fold(0.0f64, |c, e| add_cost(Some(c), e.cost_usd)),
        "cost_source": combined_source(&ends),
        "metrics": m,
    })
}

/// The one cost source of all the agents, `"mixed"` for several, null for none.
fn combined_source(ends: &[AgentEndPayload]) -> Value {
    let mut sources: Vec<&String> = vec![];
    for s in ends.iter().filter_map(|e| e.cost_source.as_ref()) {
        if !sources.contains(&s) {
            sources.push(s);
        }
    }
    match sources.len() {
        0 => Value::Null,
        1 => json!(sources[0]),
        _ => json!("mixed"),
    }
}

/// What makes two steps the same: agent, kind, tool name and arguments.
type Signature<'a> = (Option<&'a str>, EventKind, &'a Value, &'a Value);

fn signature(e: &Event) -> Signature<'_> {
    (
        e.agent.as_deref(),
        e.kind,
        &e.payload["name"],
        &e.payload["args"],
    )
}

/// Each step's seq and signature, the world's views left out.
fn steps(evs: &[Event]) -> Vec<(i64, Signature<'_>)> {
    (evs.iter().filter(|e| e.kind != EventKind::WorldView))
        .map(|e| (e.seq, signature(e)))
        .collect()
}

/// The first position at which the steps differ in `(agent, kind, name, args)`; its seq (from
/// `a`, else `b` when `a` has fewer steps). The world's views are not steps and are skipped.
/// None when identical.
pub fn first_divergent_seq(a: &[Event], b: &[Event]) -> Option<i64> {
    let (a, b) = (steps(a), steps(b));
    let differs = a.iter().zip(&b).find(|(x, y)| x.1 != y.1);
    if let Some((x, _)) = differs {
        return Some(x.0);
    }
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Less => Some(b[a.len()].0),
        std::cmp::Ordering::Greater => Some(a[b.len()].0),
        std::cmp::Ordering::Equal => None,
    }
}

/// A finished branch's final state: per part its save, and the parts of its world.
pub struct FinalState {
    pub world: Vec<WorldPart>,
    pub saves: Saves,
}

impl FinalState {
    fn save(&self, part: &str) -> Result<&str> {
        self.saves
            .get(part)
            .map(String::as_str)
            .ok_or_else(|| anyhow!("there is no final save of the part {part}"))
    }
}

/// The parts of `a`'s world that neither final state has a save of because their integration
/// keeps no saves (only replay): they can't be compared.
pub async fn not_compared(hub: &Hub, a: &FinalState, b: &FinalState) -> Result<Vec<String>> {
    let unsaved = |p: &&WorldPart| !a.saves.contains_key(&p.part) && !b.saves.contains_key(&p.part);
    let mut out = vec![];
    for p in a.world.iter().filter(unsaved) {
        if !keeps_saves(&hub.describe(&p.integration).await?.restore) {
            out.push(p.part.clone());
        }
    }
    Ok(out)
}

/// A part of `a`'s world (not in `skip`) that one of the two final states has no save of.
pub fn missing_part<'a>(a: &'a FinalState, b: &FinalState, skip: &[String]) -> Option<&'a str> {
    a.world
        .iter()
        .map(|p| p.part.as_str())
        .filter(|p| !skip.iter().any(|s| s == p))
        .find(|p| !a.saves.contains_key(*p) || !b.saves.contains_key(*p))
}

/// `[{key, a, b}]` for every key that differs between two final states, sorted: per part of
/// `a`'s world but `skip`, the `diff` of both final saves with each side's docs. Keys are
/// prefixed `part:` only when the world has more than one part.
pub async fn differing_rows(
    hub: &Hub,
    a: &FinalState,
    b: &FinalState,
    skip: &[String],
) -> Result<Vec<Value>> {
    let many = a.world.len() > 1;
    let mut out = vec![];
    for p in a.world.iter().filter(|p| !skip.contains(&p.part)) {
        let rows = part_rows(hub, p, [a.save(&p.part)?, b.save(&p.part)?]).await?;
        out.extend(rows.into_iter().map(|mut r| {
            if many {
                r["key"] = json!(format!("{}:{}", p.part, r["key"].as_str().unwrap_or("")));
            }
            r
        }));
    }
    out.sort_by(|x, y| x["key"].as_str().cmp(&y["key"].as_str()));
    Ok(out)
}

/// `[{key, a, b}]` of one part: the keys that differ between its two saves, with their docs.
/// The part's internal keys (its own bookkeeping) are left out.
async fn part_rows(hub: &Hub, p: &WorldPart, [a, b]: [&str; 2]) -> Result<Vec<Value>> {
    let mut keys = saves::diff(hub, &p.integration, a, b).await?;
    keys.retain(|k| !is_internal_key(k));
    let da = docs(hub, &p.integration, a, &keys).await?;
    let db = docs(hub, &p.integration, b, &keys).await?;
    let doc = |d: &Map<String, Value>, k: &str| d.get(k).cloned().unwrap_or(Value::Null);
    Ok(keys
        .iter()
        .map(|k| json!({"key": k, "a": doc(&da, k), "b": doc(&db, k)}))
        .collect())
}

/// The docs of `keys` in `save`; none when the part can't show some of them (keys for a whole
/// table, say), so the keys are still listed.
async fn docs(
    hub: &Hub,
    integration: &str,
    save: &str,
    keys: &[String],
) -> Result<Map<String, Value>> {
    match saves::inspect(hub, integration, save, keys).await {
        Err(e) if part_code(&e) == Some(-32602) => Ok(Map::new()),
        r => r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(seq: i64, agent: &str, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: chrono::Utc::now(),
            agent: Some(agent.into()),
            kind,
            payload,
        }
    }

    fn call(seq: i64, name: &str, args: Value) -> Event {
        ev(
            seq,
            "r",
            EventKind::ToolCall,
            json!({"name": name, "args": args}),
        )
    }

    #[test]
    fn divergence_cases() {
        let a = vec![call(1, "x", json!({"k": 1})), call(2, "y", json!({}))];
        assert_eq!(first_divergent_seq(&a, &a), None);
        let b = vec![call(1, "x", json!({"k": 1})), call(2, "y", json!({"z": 1}))];
        assert_eq!(first_divergent_seq(&a, &b), Some(2));
        assert_eq!(first_divergent_seq(&a[..1], &a), Some(2));
        assert_eq!(first_divergent_seq(&a, &a[..1]), Some(2));
        // A view only one side drew is not a step: the next step decides.
        let view = |seq| ev(seq, "r", EventKind::WorldView, json!({"seq_of": 1}));
        let viewed = vec![
            call(1, "x", json!({"k": 1})),
            view(2),
            call(3, "y", json!({})),
        ];
        assert_eq!(first_divergent_seq(&a, &viewed), None);
        assert_eq!(first_divergent_seq(&viewed, &b), Some(3));
    }

    fn branch() -> Branch {
        Branch {
            id: "b".into(),
            run_id: "r".into(),
            parent_id: None,
            start_seq: None,
            team: Team {
                id: "t".into(),
                name: "T".into(),
                agents: vec![],
            },
            changes: json!([]),
            fork_note: None,
            status: BranchStatus::Passed,
            error: None,
            created_at: chrono::Utc::now(),
            ended_at: None,
            restore_mode: None,
            restored: vec![],
        }
    }

    fn end(seq: i64, agent: &str, tokens: i64, cost: Option<f64>, source: Option<&str>) -> Event {
        ev(
            seq,
            agent,
            EventKind::AgentEnd,
            json!({"ok": true, "tokens": tokens, "cost_usd": cost, "cost_source": source}),
        )
    }

    fn start(seq: i64, agent: &str) -> Event {
        let p = json!({"name": agent, "kind": "claude_code", "model": "m"});
        ev(seq, agent, EventKind::AgentStart, p)
    }

    #[test]
    fn summary_shows_dollars_only_when_every_agent_has_them() {
        // One agent with a dollar figure, one without (Codex): tokens summed, no dollars.
        let mixed = [
            start(0, "a"),
            start(0, "b"),
            end(1, "a", 100, Some(0.5), Some("api_equivalent_estimate")),
            end(2, "b", 50, None, None),
        ];
        let s = summary(&branch(), &mixed);
        assert_eq!(s["tokens"], 150);
        assert!(s["cost_usd"].is_null());
        assert_eq!(s["cost_source"], "api_equivalent_estimate");
        // Two sources, both with dollars: summed, labelled mixed.
        let two = [
            end(1, "a", 1, Some(0.5), Some("openrouter")),
            end(2, "b", 2, Some(0.25), Some("api_equivalent_estimate")),
        ];
        let s = summary(&branch(), &two);
        assert_eq!(s["cost_usd"], 0.75);
        assert_eq!(s["cost_source"], "mixed");
        // The null does not depend on order.
        let rev = [
            end(1, "b", 50, None, None),
            end(2, "a", 100, Some(0.5), Some("openrouter")),
        ];
        assert!(summary(&branch(), &rev)["cost_usd"].is_null());
    }
}
