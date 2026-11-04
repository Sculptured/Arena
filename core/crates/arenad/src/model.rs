use chrono::{DateTime, Utc};
use parts::proto::Mode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Open,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentSpec {
    pub id: String,
    pub name: String,
    pub kind: AgentKind,
    pub model: Option<String>,
    pub prompt: String,
    /// Where an open agent's model calls go: `openrouter`, `openai`, `anthropic` or
    /// `local:<name>`. Absent (or blank) means the default provider, `openrouter`, as in runs and
    /// teams made before providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// A chain of agents; the last agent writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub agents: Vec<AgentSpec>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub kind: Option<AgentKind>,
    /// A new provider (empty or blank text: back to `openrouter`); a model change for the fork
    /// rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStatus {
    Running,
    Passed,
    Failed,
    InfraError,
    Interrupted,
}

/// `text` trimmed, `None` when that leaves nothing: what a blank provider or model means.
pub fn trimmed(text: &str) -> Option<String> {
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// A running cost total: unknown (`None`) as soon as any part of it is unknown.
pub fn add_cost(total: Option<f64>, more: Option<f64>) -> Option<f64> {
    Some(total? + more?)
}

impl BranchStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            BranchStatus::Running => "running",
            BranchStatus::Passed => "passed",
            BranchStatus::Failed => "failed",
            BranchStatus::InfraError => "infra_error",
            BranchStatus::Interrupted => "interrupted",
        }
    }

    pub fn parse(s: &str) -> Option<BranchStatus> {
        Some(match s {
            "running" => BranchStatus::Running,
            "passed" => BranchStatus::Passed,
            "failed" => BranchStatus::Failed,
            "infra_error" => BranchStatus::InfraError,
            "interrupted" => BranchStatus::Interrupted,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    RunStart,
    AgentStart,
    ModelCall,
    ToolCall,
    Handoff,
    AgentEnd,
    Check,
    WorldEvent,
}

impl EventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            EventKind::RunStart => "run_start",
            EventKind::AgentStart => "agent_start",
            EventKind::ModelCall => "model_call",
            EventKind::ToolCall => "tool_call",
            EventKind::Handoff => "handoff",
            EventKind::AgentEnd => "agent_end",
            EventKind::Check => "check",
            EventKind::WorldEvent => "world_event",
        }
    }

    pub fn parse(s: &str) -> Option<EventKind> {
        Some(match s {
            "run_start" => EventKind::RunStart,
            "agent_start" => EventKind::AgentStart,
            "model_call" => EventKind::ModelCall,
            "tool_call" => EventKind::ToolCall,
            "handoff" => EventKind::Handoff,
            "agent_end" => EventKind::AgentEnd,
            "check" => EventKind::Check,
            "world_event" => EventKind::WorldEvent,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub branch_id: String,
    pub seq: i64,
    pub at: DateTime<Utc>,
    pub agent: Option<String>,
    pub kind: EventKind,
    pub payload: Value,
}

/// Payload of `EventKind::ToolCall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallPayload {
    pub name: String,
    pub args: Value,
    pub result: Value,
    pub error: Option<String>,
    pub reads: Value,
    pub writes: Value,
    /// `saves["retail"]` when that part was saved, so the old UIs keep working. Runs recorded
    /// before integrations have only this (read as `saves: {"retail": snapshot}`).
    pub snapshot: Option<String>,
    /// Set when the write committed but saving it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_error: Option<String>,
    pub tool_call_id: Option<String>,
    /// The world part the tool ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// `{part: save}` of the parts saved after this call (None when nothing was saved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub saves: Option<std::collections::BTreeMap<String, String>>,
    /// The world clock when the call ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_ms: Option<i64>,
    /// How long the world's clock was paused for this step's saves (None when nothing was saved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_ms: Option<i64>,
}

/// Payload of `EventKind::WorldEvent`: something a part reported when its clock advanced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldEventPayload {
    pub part: String,
    pub kind: String,
    pub text: String,
    pub data: Value,
    pub clock_ms: i64,
}

/// Payload of `EventKind::ModelCall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCallPayload {
    pub request: Value,
    pub response: Value,
    pub usage: Value,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    /// The provider that answered (absent in calls recorded before providers: OpenRouter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// The provider's own response, when it differs from the canonical `response` (a provider
    /// that doesn't speak chat completions); absent when `response` is what it sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_response: Option<Value>,
}

/// Payload of `EventKind::AgentEnd`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEndPayload {
    pub ok: bool,
    pub error: Option<String>,
    pub tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub seconds: Option<f64>,
}

/// Payload of `EventKind::Handoff`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandoffPayload {
    pub note: String,
}

/// Payload of `EventKind::Check`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckPayload {
    pub passed: bool,
    /// The gold save (of the first part) the world was compared with.
    pub gold_db: Option<String>,
    /// Keys (`<table>/<id>`) whose documents differ from the gold save, sorted; prefixed
    /// `part:` when the world has more than one part.
    pub differing_rows: Option<Vec<String>>,
    /// The benchmark's named checks, for a benchmark that scores the end state itself. Then
    /// `differing_rows` lists the checks that failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<parts::proto::NamedCheck>>,
    /// `{part: save}` of every part's final state, saved before its instance stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_saves: Option<std::collections::BTreeMap<String, String>>,
    /// Parts that keep no gold save (parts with replay), so they could not be compared with
    /// the task's expected result.
    #[serde(default)]
    pub unchecked: Vec<String>,
}

/// How a part of a rerun's world was restored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RestoredHow {
    /// From its save, exactly.
    Save,
    /// By repeating its recorded inputs.
    Replay,
    /// From its save, approximately.
    Approximate,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestoredPart {
    pub part: String,
    pub how: RestoredHow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Branch {
    pub id: String,
    pub run_id: String,
    pub parent_id: Option<String>,
    pub start_seq: Option<i64>,
    pub team: Team,
    pub changes: Value,
    pub fork_note: Option<String>,
    pub status: BranchStatus,
    pub error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    /// The restore a fork asked for (None for a run's first branch).
    pub restore_mode: Option<Mode>,
    /// How each part of a fork's world was restored (empty for a run's first branch).
    pub restored: Vec<RestoredPart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub task_id: String,
    pub created_at: DateTime<Utc>,
    pub branches: Vec<Branch>,
}
