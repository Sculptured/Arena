//! The tools, through Sculptured's HTTP API. Sculptured refusing a call (bad arguments, an
//! unknown change, a change already decided) is the tool's error and changes nothing; only a
//! server that can't be reached fails the call, and an unknown tool is -32602.
use anyhow::Result;
use arena_part::proto::ToolOutput;
use arena_part::PartError;
use serde_json::{json, Value};

use crate::api::Api;
use crate::changes;

mod decide;
mod propose;

/// A tool's outcome: its output, or the refusal that is its error.
pub type Outcome = Result<ToolOutput, String>;

/// Runs tool `name` on `team`.
pub async fn call(
    api: &Api,
    team: &str,
    name: &str,
    args: &Value,
) -> Result<ToolOutput, PartError> {
    let out = match name {
        "propose_change" => propose::propose(api, team, args).await,
        "list_changes" => list(api, team).await,
        "decide_change" => decide::decide(api, team, args).await,
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

async fn list(api: &Api, team: &str) -> Result<Outcome> {
    let all = changes::fetch(api, team).await?;
    let reads = changes::by_key(&all).into_keys().collect();
    Ok(Ok(ToolOutput {
        result: Some(json!({ "changes": all })),
        reads,
        ..ToolOutput::default()
    }))
}
