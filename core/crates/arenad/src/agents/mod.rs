use std::{future::Future, sync::Arc};

use serde_json::Value;

use crate::{exec::BranchCtx, model::AgentSpec};

pub mod open;

/// One agent's turn in a branch.
pub struct AgentRun {
    /// Shared with the MCP endpoint so both use the same per-branch gate.
    pub branch: Arc<BranchCtx>,
    pub spec: AgentSpec,
    /// The first user message.
    pub input: String,
    /// A replayed message list (system message is replaced by `spec.prompt`).
    pub resume: Option<Vec<Value>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentResult {
    pub ok: bool,
    pub error: Option<String>,
    /// `{prompt_tokens, completion_tokens, total_tokens}` summed over model calls.
    pub tokens: Value,
    /// Summed cost; None if any call lacked a cost.
    pub cost_usd: Option<f64>,
    pub cost_source: Option<String>,
    pub seconds: f64,
}

pub trait Driver: Send + Sync {
    fn run(&self, ctx: AgentRun) -> impl Future<Output = AgentResult> + Send;
}
