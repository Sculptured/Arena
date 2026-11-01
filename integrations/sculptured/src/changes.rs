//! A team's proposed changes as the tools, `inspect` and `diff` show them. Keys are
//! `change/<id>`. A change is what was proposed and what became of it; the times and the agent
//! Sculptured adds are left out, so a change and its restored copy are the same.
use std::collections::BTreeMap;

use anyhow::Result;
use arena_part::proto::Docs;
use serde_json::{json, Map, Value};

use crate::api::Api;

/// The key of change `id`.
pub fn key(id: &str) -> String {
    format!("change/{id}")
}

/// A change from a row of `GET /api/teams/{team}/changes`: `id`, `action`, `system`, `summary`,
/// `payload`, `reason`, and `decision` (`{approved, by, note}`, or null while it waits).
pub fn doc(row: &Value) -> Value {
    let decision = match row.get("decision") {
        Some(d) if d.is_object() => {
            json!({"approved": d["approved"], "by": d["by"], "note": d["note"]})
        }
        _ => Value::Null,
    };
    json!({"id": row["id"], "action": row["action"], "system": row["system"],
           "summary": row["summary"], "payload": row["payload"], "reason": row["reason"],
           "decision": decision})
}

/// The team's changes, newest first.
pub async fn fetch(api: &Api, team: &str) -> Result<Vec<Value>> {
    let segments = ["teams", team, "changes"];
    let list = api
        .get(&segments, &[])
        .await?
        .expect("the team's changes")?;
    let rows = list["changes"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    Ok(rows.iter().map(doc).collect())
}

/// The change with `id` among `changes`.
pub fn find<'a>(changes: &'a [Value], id: &str) -> Option<&'a Value> {
    changes.iter().find(|c| c["id"] == id)
}

/// Changes by key.
pub type ChangeMap = BTreeMap<String, Value>;

pub fn by_key(changes: &[Value]) -> ChangeMap {
    let id = |c: &Value| c["id"].as_str().map(key);
    changes
        .iter()
        .filter_map(|c| Some((id(c)?, c.clone())))
        .collect()
}

/// `keys` (every change when empty) of `changes`; a key that is no change is null.
pub fn inspect(changes: &ChangeMap, keys: &[String]) -> Docs {
    let keys: Vec<&String> = if keys.is_empty() {
        changes.keys().collect()
    } else {
        keys.iter().collect()
    };
    let docs: Map<String, Value> = keys
        .into_iter()
        .map(|k| (k.clone(), changes.get(k).cloned().unwrap_or(Value::Null)))
        .collect();
    Docs { docs }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(decision: Option<Value>) -> Value {
        let mut r = json!({"id": "chg-1", "action": "create_purchase_order", "system": "ERP",
            "summary": "Order", "payload": {"qty": 2}, "reason": null, "at_ms": 5,
            "agent": "team.p1@arena", "agent_name": "Proposer 1", "checks": []});
        if let Some(d) = decision {
            r["decision"] = d;
        }
        r
    }

    #[test]
    fn doc_keeps_the_change_and_its_decision_only() {
        let waiting = doc(&row(None));
        assert_eq!(waiting["decision"], Value::Null);
        assert_eq!(waiting["payload"], json!({"qty": 2}));
        assert!(waiting.get("at_ms").is_none() && waiting.get("agent").is_none());
        let d = json!({"approved": false, "by": "person", "note": "no", "at_ms": 9});
        let decided = doc(&row(Some(d)));
        assert_eq!(
            decided["decision"],
            json!({"approved": false, "by": "person", "note": "no"})
        );
    }

    #[test]
    fn inspect_every_or_named_keys() {
        let map = by_key(&[doc(&row(None))]);
        assert_eq!(
            inspect(&map, &[]).docs.keys().collect::<Vec<_>>(),
            ["change/chg-1"]
        );
        let named = inspect(&map, &["change/chg-1".into(), "change/x".into()]).docs;
        assert_eq!(named["change/chg-1"]["id"], "chg-1");
        assert_eq!(named["change/x"], Value::Null);
    }
}
