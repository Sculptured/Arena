//! Branch comparison: per-branch totals, first divergent step, differing final rows.

use serde_json::{json, Value};

use crate::{model::*, runner::differing_rows};

/// `{status, seconds, tokens, cost_usd, cost_source}` for a finished branch. Only the branch's
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
    json!({
        "status": branch.status,
        "seconds": seconds,
        "tokens": ends.iter().map(|e| e.tokens.unwrap_or(0)).sum::<i64>(),
        "cost_usd": ends.iter().fold(Some(0.0f64), |c, e| add_cost(c, e.cost_usd)),
        "cost_source": combined_source(&ends),
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

fn signature(e: &Event) -> (Option<&str>, EventKind, &Value, &Value) {
    (
        e.agent.as_deref(),
        e.kind,
        &e.payload["name"],
        &e.payload["args"],
    )
}

/// The first position at which the event lists differ in `(agent, kind, name, args)`; its seq
/// (from `a`, else `b` when `a` is the shorter list). None when identical.
pub fn first_divergent_seq(a: &[Event], b: &[Event]) -> Option<i64> {
    for (x, y) in a.iter().zip(b) {
        if signature(x) != signature(y) {
            return Some(x.seq);
        }
    }
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Less => Some(b[a.len()].seq),
        std::cmp::Ordering::Greater => Some(a[b.len()].seq),
        std::cmp::Ordering::Equal => None,
    }
}

/// `[{key, a, b}]` for every row that differs between two final dumps.
pub fn row_diffs(a: &Value, b: &Value) -> Vec<Value> {
    differing_rows(a, b)
        .into_iter()
        .map(|key| {
            let (t, id) = key.split_once('/').unwrap_or((&key, ""));
            json!({"key": key, "a": a[t][id].clone(), "b": b[t][id].clone()})
        })
        .collect()
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
    }

    #[test]
    fn row_diffs_include_one_sided_rows() {
        let a = json!({"users": {"u1": {"n": 1}}, "orders": {}});
        let b = json!({"users": {"u1": {"n": 2}}, "orders": {"o": {"s": 1}}});
        let d = row_diffs(&a, &b);
        assert_eq!(d[0], json!({"key": "orders/o", "a": null, "b": {"s": 1}}));
        assert_eq!(d[1]["key"], "users/u1");
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

    #[test]
    fn summary_shows_dollars_only_when_every_agent_has_them() {
        // One agent with a dollar figure, one without (Codex): tokens summed, no dollars.
        let mixed = [
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
