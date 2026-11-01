//! Changes proposed again after a restore. A restored server withdraws the questions its
//! predecessor asked, so a change that waited can no longer be decided; the start proposes it
//! again (a new id in Sculptured) and records `original → raised` here. The tools, `inspect` and
//! `diff` show the latest raised copy under the original id and hide every raised id. The map
//! goes into saves, so restores of restores compose.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alias {
    pub original: String,
    pub raised: String,
}

/// Every re-raise, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Aliases(pub Vec<Alias>);

impl Aliases {
    /// The id in Sculptured that change `original` lives on now.
    pub fn current<'a>(&'a self, original: &'a str) -> &'a str {
        let latest = self.0.iter().rev().find(|a| a.original == original);
        latest.map_or(original, |a| a.raised.as_str())
    }

    fn is_raised(&self, id: &str) -> bool {
        self.0.iter().any(|a| a.raised == id)
    }

    pub fn add(&mut self, original: &str, raised: &str) {
        self.0.push(Alias {
            original: original.into(),
            raised: raised.into(),
        });
    }

    /// The changes as the tools show them: Sculptured's list (`raw`, docs, newest first) with
    /// each original carrying its latest raised copy, and the raised ids left out.
    pub fn present(&self, raw: &[Value]) -> Vec<Value> {
        let id = |c: &Value| c["id"].as_str().unwrap_or_default().to_string();
        let shown = raw.iter().filter(|c| !self.is_raised(&id(c)));
        shown
            .map(|c| {
                let original = id(c);
                let now = self.current(&original);
                let copy = raw.iter().find(|r| r["id"] == now).unwrap_or(c);
                let mut doc = copy.clone();
                doc["id"] = Value::String(original);
                doc
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn change(id: &str, decision: Value) -> Value {
        json!({"id": id, "action": "a", "payload": {"x": 1}, "decision": decision})
    }

    #[test]
    fn raised_copies_show_under_the_original_id() {
        let mut aliases = Aliases::default();
        aliases.add("chg-1", "chg-2");
        aliases.add("chg-1", "chg-3"); // restored again from a save of the restored instance
        assert_eq!(aliases.current("chg-1"), "chg-3");
        assert_eq!(aliases.current("chg-9"), "chg-9");
        let decided = json!({"approved": true, "by": "person", "note": null});
        let raw = [
            change("chg-3", decided.clone()),
            change("chg-2", Value::Null),
            change("chg-9", Value::Null),
            change("chg-1", Value::Null),
        ];
        let shown = aliases.present(&raw);
        assert_eq!(
            shown,
            [change("chg-9", Value::Null), change("chg-1", decided)]
        );
    }
}
