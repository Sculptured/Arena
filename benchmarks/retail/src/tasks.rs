use std::{collections::HashMap, path::Path};

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

/// The τ²-bench retail tasks and the ids offered in the demo.
pub struct Tasks {
    by_id: HashMap<String, Value>,
    demo_ids: Vec<String>,
}

impl Tasks {
    /// Reads `tasks.json` and `demo_tasks.json` from `data_dir`.
    pub fn load(data_dir: &Path) -> Result<Tasks> {
        let read = |name: &str| -> Result<Value> {
            let p = data_dir.join(name);
            let text =
                std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display()))?;
            serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))
        };
        let tasks = read("tasks.json")?;
        let by_id = tasks
            .as_array()
            .ok_or_else(|| anyhow!("tasks.json is not an array"))?
            .iter()
            .filter_map(|t| t["id"].as_str().map(|id| (id.to_string(), t.clone())))
            .collect();
        let demo_ids =
            serde_json::from_value(read("demo_tasks.json")?).context("demo_tasks.json")?;
        Ok(Tasks { by_id, demo_ids })
    }

    /// `(id, reason)` of each demo task.
    pub fn demo(&self) -> Vec<(String, String)> {
        self.demo_ids
            .iter()
            .filter_map(|id| {
                let t = self.by_id.get(id)?;
                let reason = t["user_scenario"]["instructions"]["reason_for_call"]
                    .as_str()
                    .unwrap_or("");
                Some((id.clone(), reason.to_string()))
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<&Value> {
        self.by_id.get(id)
    }

    /// The gold actions `(name, arguments)` of a task: `evaluation_criteria.actions`.
    pub fn gold_actions(&self, id: &str) -> Option<Vec<(String, Value)>> {
        let t = self.get(id)?;
        Some(
            t["evaluation_criteria"]["actions"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|x| {
                            (
                                x["name"].as_str().unwrap_or("").to_string(),
                                x["arguments"].clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        )
    }

    /// The customer ticket text (`gap.py::ticket`, verbatim).
    pub fn ticket(&self, id: &str) -> Option<String> {
        let ins = &self.get(id)?["user_scenario"]["instructions"];
        let text = |k: &str| match &ins[k] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let or = |s: String, d: &str| if s.is_empty() { d.to_string() } else { s };
        Some(format!(
            "# Customer ticket\n\n**Reason for contact:** {}\n\n\
             **What the customer told us about themselves:** {}\n\n\
             **What the customer does not know:** {}\n\n\
             **Notes on the customer:** {}\n\n\
             The customer has confirmed in advance that every change described above may be made.\n",
            text("reason_for_call"),
            text("known_info"),
            or(text("unknown_info"), "nothing noted"),
            text("task_instructions"),
        ))
    }
}
