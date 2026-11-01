//! The tools, through Sculptured's HTTP API. Sculptured refusing a call (bad arguments, an
//! unknown change, a change already decided) is the tool's error and changes nothing; only a
//! server that can't be reached fails the call, and an unknown tool is -32602.
use anyhow::Result;
use arena_part::proto::ToolOutput;
use arena_part::PartError;
use serde_json::{json, Value};

use crate::aliases::Aliases;
use crate::api::Api;
use crate::changes;

mod decide;
mod propose;

pub use propose::propose;

/// The team the tools work on: its server, its name, and the changes proposed again.
pub struct Team<'a> {
    pub api: &'a Api,
    pub name: &'a str,
    pub aliases: &'a Aliases,
}

impl Team<'_> {
    /// The team's changes as the tools show them, newest first.
    pub async fn changes(&self) -> Result<Vec<Value>> {
        let raw = changes::fetch(self.api, self.name).await?;
        Ok(self.aliases.present(&raw))
    }
}

/// A tool's outcome: its output, or the refusal that is its error.
pub type Outcome = Result<ToolOutput, String>;

/// Runs tool `name` on team `t`.
pub async fn call(t: &Team<'_>, name: &str, args: &Value) -> Result<ToolOutput, PartError> {
    let out = match name {
        "propose_change" => propose(t.api, t.name, args).await,
        "list_changes" => list(t).await,
        "decide_change" => decide::decide(t, args).await,
        _ => return Err(PartError::invalid(format!("unknown tool {name}"))),
    };
    Ok(match out.map_err(PartError::internal_from)? {
        Ok(o) => o,
        Err(e) => ToolOutput {
            error: Some(e),
            ..ToolOutput::default()
        },
    })
}

/// The output of a tool that wrote `change`.
fn wrote(change: Value) -> ToolOutput {
    let key = change["id"].as_str().map(changes::key);
    ToolOutput {
        result: Some(change),
        writes: key.into_iter().collect(),
        ..ToolOutput::default()
    }
}

async fn list(t: &Team<'_>) -> Result<Outcome> {
    let all = t.changes().await?;
    let reads = changes::by_key(&all).into_keys().collect();
    Ok(Ok(ToolOutput {
        result: Some(json!({ "changes": all })),
        reads,
        ..ToolOutput::default()
    }))
}
