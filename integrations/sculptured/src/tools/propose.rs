//! `propose_change`: proposes the change to the team as an outside system, through Sculptured's
//! `POST /api/teams/{team}/changes` (proposer `arena`). Sculptured checks the team's rules,
//! records the change and asks a person in Needs you; the person's answer (`decide_change`)
//! decides it. Sculptured refusing the arguments is the tool's error.
use anyhow::{Context, Result};
use arena_part::proto::{ToolOutput, WorldEvent};
use reqwest::Method;
use serde_json::Value;

use super::{wrote, Outcome};
use crate::api::{Api, Reply};
use crate::changes;
use crate::team::MAX_WAITING;

/// The name the tools propose under: Sculptured shows it as `external.arena@<team>`.
pub const PROPOSER: &str = "arena";

pub async fn propose(api: &Api, team: &str, args: &Value) -> Result<Outcome> {
    if !args.is_object() {
        return Ok(Err("the arguments must be an object".into()));
    }
    let all = changes::fetch(api, team).await?;
    if all.iter().filter(|c| c["decision"].is_null()).count() >= MAX_WAITING {
        let busy = format!("{MAX_WAITING} changes already wait for a decision; decide one first");
        return Ok(Err(busy));
    }
    let segments = ["teams", team, "changes"];
    let id = match api.send(Method::POST, &segments, body(args)).await? {
        Reply::Ok(v) => v["proposal_id"]
            .as_str()
            .context("Sculptured gave no proposal id")?
            .to_string(),
        Reply::Refused { message, .. } => return Ok(Err(message)),
    };
    let all = changes::fetch(api, team).await?;
    let change = changes::find(&all, &id).with_context(|| format!("{id} is not listed"))?;
    Ok(Ok(proposed(change.clone())))
}

/// The request: the tool's arguments as they are, from the tools' proposer.
fn body(args: &Value) -> Value {
    let mut body = args.clone();
    body["proposer"] = Value::String(PROPOSER.into());
    body
}

/// The output of a proposal: the change written, and a `change_proposed` event whose data is the
/// change, so a world's links can hand it to whoever decides it (a simulated person, say).
fn proposed(change: Value) -> ToolOutput {
    let id = change["id"].as_str().unwrap_or_default();
    let text = format!(
        "change {id} proposed: {}",
        change["summary"].as_str().unwrap_or_default()
    );
    let event = WorldEvent {
        kind: "change_proposed".into(),
        text,
        data: change.clone(),
    };
    ToolOutput {
        events: Box::new([event]),
        ..wrote(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_proposal_gives_an_event_with_the_change() {
        let change = json!({"id": "chg-1", "action": "batch_release", "summary": "Release",
                            "payload": {"batch": "B"}, "decision": null});
        let out = proposed(change.clone());
        assert_eq!(out.writes, ["change/chg-1"]);
        let e = &out.events[0];
        assert_eq!((e.kind.as_str(), &e.data), ("change_proposed", &change));
        assert_eq!(e.text, "change chg-1 proposed: Release");
    }

    #[test]
    fn the_request_is_the_arguments_from_the_tools_proposer() {
        let args = json!({"action": "a", "summary": "s", "payload": {"x": 1}, "proposer": "me"});
        let b = body(&args);
        assert_eq!(
            b["proposer"], PROPOSER,
            "the arguments cannot pick another proposer"
        );
        assert_eq!(
            (&b["action"], &b["payload"]),
            (&args["action"], &args["payload"])
        );
    }
}
