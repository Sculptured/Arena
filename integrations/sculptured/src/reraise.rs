//! A start from a save proposes again every change that waited in it: the restored server
//! withdrew their questions, so nothing could decide them otherwise. The copies get new ids in
//! Sculptured; `aliases` shows them under the original ids.
use anyhow::{anyhow, Context, Result};
use serde_json::Value;

use crate::aliases::Aliases;
use crate::api::Api;
use crate::tools::propose;

/// The changes of a save (as the tools showed them) still waiting for a decision.
pub fn waiting(saved: &[Value]) -> Vec<&Value> {
    saved.iter().filter(|c| c["decision"].is_null()).collect()
}

/// Proposes each waiting change of `saved` again, oldest first, exactly as it was proposed;
/// returns `aliases` with the copies added.
pub async fn reraise(api: &Api, team: &str, saved: &[Value], aliases: Aliases) -> Result<Aliases> {
    let mut aliases = aliases;
    for change in waiting(saved).into_iter().rev() {
        let original = change["id"].as_str().context("a saved change has no id")?;
        let out = propose(api, team, &proposal(change)).await?;
        let out = out.map_err(|why| anyhow!("proposing {original} again: {why}"))?;
        let raised = out.result.as_ref().and_then(|c| c["id"].as_str());
        aliases.add(original, raised.context("a proposal gave no id")?);
    }
    Ok(aliases)
}

/// The arguments that propose `change` again: everything it was proposed with.
fn proposal(change: &Value) -> Value {
    let mut args = change.clone();
    if let Some(o) = args.as_object_mut() {
        o.remove("id");
        o.remove("decision");
        o.retain(|_, v| !v.is_null());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_waiting_change_is_proposed_as_it_was() {
        let waits = json!({"id": "chg-1", "action": "a", "system": null, "summary": "s",
                           "payload": {"x": 1}, "reason": "r", "decision": null});
        let done = json!({"id": "chg-2", "decision": {"approved": true}});
        let saved = [waits.clone(), done];
        assert_eq!(waiting(&saved), [&waits]);
        let args = json!({"action": "a", "summary": "s", "payload": {"x": 1}, "reason": "r"});
        assert_eq!(proposal(&waits), args);
    }
}
