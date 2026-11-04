//! Wire types of arena-part/1. See docs/protocol.md.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROTOCOL: u32 = 1;

fn all(s: &str, ok: impl Fn(u8) -> bool, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(ok)
}

/// Instance and save ids: `^[a-z0-9_]{1,63}$`.
pub fn valid_id(s: &str) -> bool {
    all(
        s,
        |b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_',
        63,
    )
}

/// A `gc` keep entry: a save id, or an id prefix followed by `*` (`base_*` keeps every save whose
/// id starts with `base_`).
pub fn valid_keep(s: &str) -> bool {
    match s.strip_suffix('*') {
        Some(prefix) => prefix.is_empty() || valid_id(prefix),
        None => valid_id(s),
    }
}

/// True when `keep` (`gc` entries, see `valid_keep`) keeps the save `id`.
pub fn kept<S: AsRef<str>>(keep: &[S], id: &str) -> bool {
    keep.iter().any(|k| match k.as_ref().strip_suffix('*') {
        Some(prefix) => id.starts_with(prefix),
        None => k.as_ref() == id,
    })
}

/// Package names: `^[a-z0-9-]{1,40}$`.
pub fn valid_package_name(s: &str) -> bool {
    all(
        s,
        |b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-',
        40,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExactKind {
    Save,
    Replay,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restore {
    pub exact: ExactKind,
    pub approximate: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub effect: Effect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Describe {
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub restore: Restore,
    pub uses_clock: bool,
    pub fingerprint: Value,
    pub tools: Vec<ToolSchema>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Exact,
    Approximate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FromSave {
    pub save: String,
    pub mode: Mode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartParams {
    pub instance: String,
    #[serde(default)]
    pub config: Value,
    pub seed: u64,
    pub clock_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<FromSave>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    pub instance: String,
    pub name: String,
    #[serde(default)]
    pub args: Value,
    pub clock_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
    /// Events the call itself gives the world: recorded, and matched against the world's links.
    #[serde(default, skip_serializing_if = "<[_]>::is_empty")]
    pub events: Box<[WorldEvent]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdvanceParams {
    pub instance: String,
    pub clock_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldEvent {
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub data: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Advanced {
    pub changed: bool,
    #[serde(default)]
    pub events: Vec<WorldEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveParams {
    pub instance: String,
    pub save: String,
    pub dir: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub bytes: u64,
    pub exact: bool,
}

/// What `inspect` and `diff` look at: a live instance or a save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    Instance { instance: String },
    Save { save: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectParams {
    pub target: Target,
    #[serde(default)]
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Docs {
    pub docs: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffParams {
    pub a: Target,
    pub b: Target,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Differing {
    pub differing: Vec<String>,
}

impl Differing {
    /// The keys whose docs differ between `a` and `b` (a key only one side has differs), sorted.
    pub fn between<V: PartialEq>(a: &BTreeMap<String, V>, b: &BTreeMap<String, V>) -> Differing {
        let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
        let differing = keys
            .into_iter()
            .filter(|k| a.get(*k) != b.get(*k))
            .cloned()
            .collect();
        Differing { differing }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdParams {
    pub instance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveId {
    pub save: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GcParams {
    pub keep: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcResult {
    pub dropped: u64,
    pub bytes: u64,
}

// ---- Benchmark side ----

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldPart {
    pub part: String,
    pub integration: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub reason: String,
    pub ticket: String,
}

/// One agent of a benchmark's suggested team: its id (`^[a-z0-9_-]+$`), name and system prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamRole {
    pub id: String,
    pub name: String,
    pub prompt: String,
}

/// A link between two parts: an event of `from` of kind `event` calls `to`'s `tool`. In `args`, a
/// string starting with `/` is a JSON pointer into the event's `data`; anything else is a constant.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub event: String,
    pub to: String,
    pub tool: String,
    #[serde(default)]
    pub args: Map<String, Value>,
}

/// The team a benchmark suggests: its agents in the order they run (the last one writes).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchTeam {
    pub id: String,
    pub name: String,
    pub agents: Vec<TeamRole>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchDescribe {
    pub name: String,
    pub version: String,
    pub protocol: u32,
    pub world: Vec<WorldPart>,
    pub tasks: Vec<TaskInfo>,
    pub instructions: String,
    /// The heading over `instructions` in each agent's input; arenad uses a generic one if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions_title: Option<String>,
    /// The team the benchmark suggests; arenad suggests a generic one if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<BenchTeam>,
    pub tools: Vec<ToolSchema>,
    /// Calls between the parts of the world.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedParams {
    pub connections: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchCallTool {
    pub name: String,
    #[serde(default)]
    pub args: Value,
    pub connections: Map<String, Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchToolOutput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub part: String,
    #[serde(default)]
    pub reads: Vec<String>,
    #[serde(default)]
    pub writes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoldParams {
    pub task: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GoldActions {
    pub actions: Vec<Action>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("snap_1"));
        assert!(!valid_id("Bad Id"));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(64)));
        assert!(valid_package_name("ridgeline-plant"));
        assert!(!valid_package_name("a_b"));
    }

    #[test]
    fn keep_entries() {
        assert!(valid_keep("s_1") && valid_keep("base_*") && valid_keep("*"));
        assert!(!valid_keep("Base_*") && !valid_keep("a*b") && !valid_keep("**"));
        let keep = ["s_1", "gold_*"];
        assert!(kept(&keep, "s_1") && kept(&keep, "gold_x_1"));
        assert!(!kept(&keep, "s_12") && !kept(&keep, "xgold_1"));
    }

    #[test]
    fn differing_between_maps() {
        let a: BTreeMap<String, i32> = [("x".into(), 1), ("y".into(), 2)].into();
        let b: BTreeMap<String, i32> = [("y".into(), 3), ("z".into(), 4), ("x".into(), 1)].into();
        assert_eq!(Differing::between(&a, &b).differing, ["y", "z"]);
        assert!(Differing::between(&a, &a).differing.is_empty());
    }

    #[test]
    fn target_untagged() {
        let t: Target = serde_json::from_str(r#"{"save":"s1"}"#).unwrap();
        assert_eq!(t, Target::Save { save: "s1".into() });
    }
}
