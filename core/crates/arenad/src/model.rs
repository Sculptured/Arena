use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub snapshot: Option<String>,
    /// Set when the write committed but snapshotting it failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_error: Option<String>,
    pub tool_call_id: Option<String>,
}

/// Payload of `EventKind::ModelCall`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCallPayload {
    pub request: Value,
    pub response: Value,
    pub usage: Value,
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
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
    pub gold_db: Option<String>,
    pub differing_rows: Option<i64>,
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
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub task_id: String,
    pub created_at: DateTime<Utc>,
    pub branches: Vec<Branch>,
}
