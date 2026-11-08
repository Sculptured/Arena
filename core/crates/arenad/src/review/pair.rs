//! The digest of two branches compared: each branch's digest (smaller than a lone run's), and
//! what Arena's comparison found, summarised: the checks whose outcome differs, the first step
//! where they diverge, the metrics side by side and the final worlds' differences by kind.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::{
    digest::{digest, Capped, Digest, RunRecord},
    text::cut_json,
};

/// The bound on each side's digest in a comparison.
pub const SIDE_BYTES: usize = 26_000;
/// How many changed rows of the final worlds the digest shows, and fields of each.
const ROW_SAMPLE: usize = 12;
const FIELDS: usize = 8;
const VALUE_BYTES: usize = 120;

/// What `GET /api/compare` found for the pair.
pub struct CompareFacts<'a> {
    pub first_divergent_seq: Option<i64>,
    /// `[{key, a, b}]`.
    pub differing_rows: &'a [Value],
    pub not_compared: &'a [String],
}

#[derive(Debug, Clone, Serialize)]
pub struct PairDigest {
    pub a: Digest,
    pub b: Digest,
    pub comparison: PairFacts,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairFacts {
    /// The first step at which the two branches did something different.
    pub first_divergent_seq: Option<i64>,
    /// The checks whose outcome differs between A and B.
    pub outcome_differs: Vec<OutcomeRow>,
    pub outcome_same: usize,
    pub metrics: SideBySide,
    pub world: WorldDiff,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeRow {
    pub check: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub safety: bool,
    pub a: Option<bool>,
    pub b: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SideBySide {
    pub seconds: [f64; 2],
    pub tokens: [i64; 2],
    pub cost_usd: [Option<f64>; 2],
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldDiff {
    pub differing_rows: usize,
    /// Per kind of row (`part:kind`): rows only A has, only B has, and rows both have that differ.
    pub by_kind: Vec<KindCount>,
    /// Some rows both have that differ, with the fields that differ (`[a, b]`).
    pub changed: Capped<ChangedRow>,
    #[serde(skip_serializing_if = "<[String]>::is_empty")]
    pub not_compared: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct KindCount {
    pub kind: String,
    pub only_a: usize,
    pub only_b: usize,
    pub changed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangedRow {
    pub key: String,
    pub fields: BTreeMap<String, [String; 2]>,
}

pub fn pair_digest(a: &RunRecord, b: &RunRecord, facts: &CompareFacts) -> PairDigest {
    let (da, db) = (digest(a, SIDE_BYTES), digest(b, SIDE_BYTES));
    let (outcome_differs, outcome_same) = outcome(&da, &db);
    let comparison = PairFacts {
        first_divergent_seq: facts.first_divergent_seq,
        outcome_differs,
        outcome_same,
        metrics: SideBySide {
            seconds: [da.metrics.seconds, db.metrics.seconds],
            tokens: [da.metrics.tokens, db.metrics.tokens],
            cost_usd: [da.metrics.cost_usd, db.metrics.cost_usd],
        },
        world: world_diff(facts),
    };
    PairDigest {
        a: da,
        b: db,
        comparison,
    }
}

/// The checks whose outcome differs (by name, A's order then B's), and how many are the same.
fn outcome(a: &Digest, b: &Digest) -> (Vec<OutcomeRow>, usize) {
    let items = |d: &Digest| {
        d.checks
            .as_ref()
            .map(|c| c.items.items.clone())
            .unwrap_or_default()
    };
    let (ia, ib) = (items(a), items(b));
    let mut names: Vec<(String, bool)> = ia.iter().map(|c| (c.name.clone(), c.safety)).collect();
    for c in &ib {
        if !names.iter().any(|(n, _)| n == &c.name) {
            names.push((c.name.clone(), c.safety));
        }
    }
    let of = |list: &[super::digest::CheckLine], n: &str| {
        list.iter().find(|c| c.name == n).map(|c| c.passed)
    };
    let rows: Vec<OutcomeRow> = names
        .into_iter()
        .map(|(check, safety)| OutcomeRow {
            a: of(&ia, &check),
            b: of(&ib, &check),
            check,
            safety,
        })
        .collect();
    let total = rows.len();
    let differs: Vec<OutcomeRow> = rows.into_iter().filter(|r| r.a != r.b).collect();
    let same = total - differs.len();
    (differs, same)
}

/// `part:kind` of a key `part:kind/id` (or `kind/id`).
fn kind_of(key: &str) -> &str {
    key.split_once('/').map_or(key, |(k, _)| k)
}

fn world_diff(facts: &CompareFacts) -> WorldDiff {
    let mut kinds: BTreeMap<String, KindCount> = BTreeMap::new();
    let mut changed = vec![];
    for r in facts.differing_rows {
        let key = r["key"].as_str().unwrap_or("");
        let k = kinds.entry(kind_of(key).to_string()).or_default();
        match (r["a"].is_null(), r["b"].is_null()) {
            (false, true) => k.only_a += 1,
            (true, false) => k.only_b += 1,
            _ => {
                k.changed += 1;
                changed.push(changed_row(key, &r["a"], &r["b"]));
            }
        }
    }
    let by_kind = kinds
        .into_iter()
        .map(|(kind, c)| KindCount { kind, ..c })
        .collect();
    WorldDiff {
        differing_rows: facts.differing_rows.len(),
        by_kind,
        changed: Capped::first(changed, ROW_SAMPLE),
        not_compared: facts.not_compared.to_vec(),
    }
}

fn changed_row(key: &str, a: &Value, b: &Value) -> ChangedRow {
    let null = Value::Null;
    let (oa, ob) = (a.as_object(), b.as_object());
    let mut names: Vec<&String> = oa.into_iter().flat_map(|m| m.keys()).collect();
    names.extend(ob.into_iter().flat_map(|m| m.keys()));
    names.sort();
    names.dedup();
    let field = |m: Option<&serde_json::Map<String, Value>>, n: &str| {
        m.and_then(|m| m.get(n)).unwrap_or(&null).clone()
    };
    let fields = names
        .into_iter()
        .map(|n| (n, field(oa, n), field(ob, n)))
        .filter(|(_, x, y)| x != y)
        .take(FIELDS)
        .map(|(n, x, y)| {
            (
                n.clone(),
                [cut_json(&x, VALUE_BYTES), cut_json(&y, VALUE_BYTES)],
            )
        })
        .collect();
    ChangedRow {
        key: key.to_string(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::digest::{
        size_of,
        tests::{single_run, team_run, TICKET},
    };

    #[test]
    fn the_recorded_comparison_is_summarised_by_kind_and_outcome() {
        let cmp: Value =
            serde_json::from_str(include_str!("testdata/compare_team_vs_single.json")).unwrap();
        let rows = cmp["differing_rows"].as_array().unwrap();
        let (ta, sb) = (team_run(), single_run());
        let ra = RunRecord {
            branch: &ta.branch,
            events: &ta.events,
            ticket: Some(TICKET),
        };
        let rb = RunRecord {
            branch: &sb.branch,
            events: &sb.events,
            ticket: Some(TICKET),
        };
        let facts = CompareFacts {
            first_divergent_seq: cmp["first_divergent_seq"].as_i64(),
            differing_rows: rows,
            not_compared: &[],
        };
        let d = pair_digest(&ra, &rb, &facts);
        assert!(size_of(&d) <= 2 * SIDE_BYTES + 12_000, "{}", size_of(&d));
        assert_eq!(d.comparison.first_divergent_seq, Some(3));
        let differs: Vec<&str> = d
            .comparison
            .outcome_differs
            .iter()
            .map(|r| r.check.as_str())
            .collect();
        // The single agent passed four checks the team failed; both missed the next batch.
        assert_eq!(
            differs,
            [
                "deviation_opened",
                "allocated_earliest_due_first",
                "customers_notified",
                "crm_updated"
            ]
        );
        assert_eq!(d.comparison.outcome_same, 10);
        let w = &d.comparison.world;
        assert_eq!(w.differing_rows, 137);
        let total: usize = w
            .by_kind
            .iter()
            .map(|k| k.only_a + k.only_b + k.changed)
            .sum();
        assert_eq!(total, 137);
        assert!(w.by_kind.iter().any(|k| k.kind == "crm:note"));
        assert!(w.changed.items.len() <= ROW_SAMPLE);
        assert!(w
            .changed
            .items
            .iter()
            .all(|r| !r.fields.is_empty() && r.fields.len() <= FIELDS));
    }
}
