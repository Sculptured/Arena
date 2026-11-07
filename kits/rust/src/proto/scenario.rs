//! A run's scenario: the disruptions a benchmark offers a scenario builder (`describe.
//! scenario_options`), and the choices a run or suite makes of them. See docs/protocol.md,
//! "A run's scenario".
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What a benchmark offers a scenario builder: the tasks a scenario may start from and the
/// disruptions it may add.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioOptions {
    /// The task ids a scenario may start from.
    pub base: Vec<String>,
    pub disruptions: Vec<DisruptionOption>,
    /// Per part, the setup call arenad adds to a task that has none for that part, to carry the
    /// part's disruptions (the part's scenario tool, with its base arguments).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub setup: BTreeMap<String, SetupTemplate>,
}

/// The setup call arenad makes on `part` to carry disruptions when a task has none for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupTemplate {
    pub tool: String,
    #[serde(default)]
    pub args: Value,
}

/// One disruption a builder may add: shown as `label` and `description`, it is `template` with
/// its `{param}` placeholders filled in, appended to the setup args of `part`'s scenario call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisruptionOption {
    pub id: String,
    pub label: String,
    pub description: String,
    pub part: String,
    /// True: the builder offers it selected.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
    #[serde(default)]
    pub params: Vec<ScenarioParam>,
    /// A disruption (`{id, when, do, text}`); string values may contain `{param}`. A string that
    /// is exactly one placeholder takes the param's value as it is (a number stays a number).
    pub template: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    Int,
    Choice,
}

/// A knob of a disruption: an integer (within `min` and `max`) or one of `choices`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioParam {
    pub name: String,
    pub label: String,
    pub kind: ParamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    pub default: Value,
}

/// The scenario a run or suite asks for: `{disruptions: [{id, params}]}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenarioRequest {
    #[serde(default)]
    pub disruptions: Vec<ChosenDisruption>,
}

/// One disruption chosen: an option's `id` and the values of its params (defaults fill the rest).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChosenDisruption {
    pub id: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}
