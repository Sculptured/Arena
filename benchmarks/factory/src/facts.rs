//! What a correct run must have got right, read from the factory example's data file: the press,
//! its part, the vendors, the skilled technicians and the customers with orders on its line.
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// Next to the arena checkout, from the package folder the hub runs the program in.
pub const DEFAULT_DATA: &str = "../../../Sculptured-Examples/factory/data/ridgeline.json";

/// The press that fails.
pub const MACHINE: &str = "PR-A1";

#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub machine: String,
    pub part: String,
    pub approved_vendors: Vec<String>,
    pub technicians: Vec<String>,
    pub customers: Vec<String>,
}

pub fn read_data(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path).with_context(|| {
        format!(
            "reading {} (the factory example is not public yet; see benchmarks/factory/README.md)",
            path.display()
        )
    })?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn list<'a>(data: &'a Value, name: &str) -> impl Iterator<Item = &'a Value> {
    data[name].as_array().into_iter().flatten()
}

fn text(v: &Value, field: &str) -> Option<String> {
    v[field].as_str().map(str::to_string)
}

/// The document of the press that fails.
pub fn machine(data: &Value) -> Result<&Value> {
    list(data, "machines")
        .find(|m| m["id"] == MACHINE)
        .ok_or_else(|| anyhow!("the data has no machine {MACHINE}"))
}

fn has_skill(t: &Value, skill: &str) -> bool {
    t["skills"]
        .as_array()
        .is_some_and(|s| s.iter().any(|x| x == skill))
}

impl Facts {
    pub fn of(data: &Value) -> Result<Facts> {
        let press = machine(data)?;
        let part = text(press, "spindle_part").ok_or_else(|| anyhow!("{MACHINE} has no part"))?;
        let kind = press["kind"].as_str().unwrap_or_default();
        let on_line = |o: &&Value| o["line"] == press["line"];
        let mut customers: Vec<String> = list(data, "orders")
            .filter(on_line)
            .filter_map(|o| text(o, "customer"))
            .collect();
        customers.sort();
        customers.dedup();
        let approved = list(data, "suppliers").filter(|s| s["approved"] == true);
        let skilled =
            list(data, "technicians").filter(|t| has_skill(t, "spindle") && has_skill(t, kind));
        Ok(Facts {
            machine: MACHINE.to_string(),
            part,
            approved_vendors: approved.filter_map(|s| text(s, "name")).collect(),
            technicians: skilled.filter_map(|t| text(t, "id")).collect(),
            customers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn facts_come_from_the_data() {
        let data = json!({
            "machines": [{"id": "PR-A1", "line": "L", "kind": "press", "spindle_part": "P-1"}],
            "suppliers": [{"name": "Good", "approved": true}, {"name": "Bad", "approved": false}],
            "technicians": [{"id": "T-1", "skills": ["spindle", "press"]},
                            {"id": "T-2", "skills": ["spindle"]}],
            "orders": [{"customer": "B", "line": "L"}, {"customer": "A", "line": "L"},
                       {"customer": "A", "line": "L"}, {"customer": "C", "line": "M"}],
        });
        let want = Facts {
            machine: "PR-A1".into(),
            part: "P-1".into(),
            approved_vendors: vec!["Good".into()],
            technicians: vec!["T-1".into()],
            customers: vec!["A".into(), "B".into()],
        };
        assert_eq!(Facts::of(&data).unwrap(), want);
        assert!(Facts::of(&json!({})).is_err());
    }
}
