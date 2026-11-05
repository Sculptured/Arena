//! Documents read from saves, and the diffs between saves, kept so a `diff` or `inspect` asked
//! again needn't build scratch sites from the same saves (that takes tens of seconds). A save never
//! changes, so what was read from it stays true. Bounded, and dropped with its save.
use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Map, Value};

/// Saves whose documents are kept; past this the cache starts over.
const MAX_SAVES: usize = 16;

#[derive(Default)]
pub(crate) struct DocCache {
    by_save: Mutex<HashMap<String, Map<String, Value>>>,
    diffs: Mutex<HashMap<(String, String), Vec<String>>>,
}

impl DocCache {
    pub(crate) fn put(&self, save: &str, docs: Map<String, Value>) {
        let mut m = self.by_save.lock().expect("doc cache lock");
        if m.len() >= MAX_SAVES && !m.contains_key(save) {
            m.clear();
        }
        m.entry(save.to_string()).or_default().extend(docs);
    }

    /// The documents of `keys` in `save`, when every one of them is kept.
    pub(crate) fn get(&self, save: &str, keys: &[String]) -> Option<Map<String, Value>> {
        let m = self.by_save.lock().expect("doc cache lock");
        let kept = m.get(save)?;
        keys.iter()
            .map(|k| kept.get(k).map(|d| (k.clone(), d.clone())))
            .collect()
    }

    pub(crate) fn drop_save(&self, save: &str) {
        self.by_save.lock().expect("doc cache lock").remove(save);
        let mut d = self.diffs.lock().expect("doc cache lock");
        d.retain(|(a, b), _| a != save && b != save);
    }

    pub(crate) fn put_diff(&self, a: &str, b: &str, differing: &[String]) {
        let mut d = self.diffs.lock().expect("doc cache lock");
        if d.len() >= MAX_SAVES * MAX_SAVES {
            d.clear();
        }
        d.insert((a.to_string(), b.to_string()), differing.to_vec());
    }

    /// The keys that differ between saves `a` and `b`, when that diff was made before.
    pub(crate) fn diff(&self, a: &str, b: &str) -> Option<Vec<String>> {
        let d = self.diffs.lock().expect("doc cache lock");
        d.get(&(a.to_string(), b.to_string())).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn docs(pairs: &[(&str, i64)]) -> Map<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect()
    }

    #[test]
    fn a_save_answers_only_when_it_keeps_every_key_and_forgets_when_dropped() {
        let c = DocCache::default();
        c.put("s1", docs(&[("Item/A", 1), ("Item/B", 2)]));
        let keys = |ks: &[&str]| ks.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(
            c.get("s1", &keys(&["Item/B"])),
            Some(docs(&[("Item/B", 2)]))
        );
        assert_eq!(c.get("s1", &keys(&["Item/B", "Item/C"])), None);
        assert_eq!(c.get("s2", &keys(&["Item/A"])), None);
        c.drop_save("s1");
        assert_eq!(c.get("s1", &keys(&["Item/A"])), None);
    }

    #[test]
    fn a_diff_is_kept_per_ordered_pair_and_forgotten_with_either_save() {
        let c = DocCache::default();
        c.put_diff("a", "b", &["Item/A".to_string()]);
        assert_eq!(c.diff("a", "b"), Some(vec!["Item/A".to_string()]));
        assert_eq!(c.diff("b", "a"), None);
        c.drop_save("b");
        assert_eq!(c.diff("a", "b"), None);
    }

    #[test]
    fn past_its_bound_it_starts_over() {
        let c = DocCache::default();
        for i in 0..MAX_SAVES {
            c.put(&format!("s{i}"), docs(&[("K/x", 1)]));
        }
        c.put("new", docs(&[("K/x", 2)]));
        assert_eq!(c.get("s0", &["K/x".to_string()]), None);
        assert!(c.get("new", &["K/x".to_string()]).is_some());
    }
}
