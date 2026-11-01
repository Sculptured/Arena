//! `propose_change`: a free Practice agent of the team is told `propose_change:<args>`, and calls
//! Sculptured's `propose_change` agent tool with them. Sculptured records the change and asks a
//! person, and the agent's call waits for the answer, so the tool returns once that question is
//! asked. Sculptured refusing the arguments ends the agent's turn with its reason instead.
use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde_json::{json, Value};

use super::{wrote, Outcome};
use crate::api::Api;
use crate::changes;
use crate::poll;
use crate::team::{self, PROPOSERS};

/// How a Practice agent starts the reply of a tool call that failed.
const FAILED: &str = "Practice collaboration failed: ";

pub async fn propose(api: &Api, team: &str, args: &Value) -> Result<Outcome> {
    if !args.is_object() {
        return Ok(Err("the arguments must be an object".into()));
    }
    let Some(seat) = free_seat(api, team).await? else {
        let busy = format!("{PROPOSERS} changes already wait for a decision; decide one first");
        return Ok(Err(busy));
    };
    let after = last_seq(api, &seat).await?;
    let text = format!("propose_change:{args}");
    let body = json!({"text": text, "intent": "instruct"});
    let sent = api
        .send(Method::POST, &["seats", &seat, "send"], body)
        .await?;
    let sent = sent.expect("telling an agent to propose")?;
    let message = sent["message_id"].as_str().context("no message id")?;
    let id = match asked(api, &seat, after, message).await? {
        Asked::Question(id) => id,
        Asked::Refused(why) => return Ok(Err(why)),
    };
    let all = changes::fetch(api, team).await?;
    let change = changes::find(&all, &id).with_context(|| format!("{id} is not listed"))?;
    Ok(Ok(wrote(change.clone())))
}

/// A running agent of the team with no change waiting for a decision.
async fn free_seat(api: &Api, team: &str) -> Result<Option<String>> {
    let seats = team::seats(api, team).await?;
    if !seats.iter().any(team::Seat::running) {
        bail!("no agent of team {team} is running");
    }
    let free = seats
        .into_iter()
        .find(|s| s.running() && s.open_questions == 0);
    Ok(free.map(|s| s.id))
}

/// The seq of the agent's latest event (0 when it has none).
async fn last_seq(api: &Api, seat: &str) -> Result<i64> {
    let tail = [("tail", "1".to_string())];
    let events = api.get(&["seats", seat, "events"], &tail).await?;
    let events = events.expect("the agent's events")?;
    Ok(events[0]["seq"].as_i64().unwrap_or(0))
}

/// What became of the message: Sculptured asked about change `id`, or refused it.
enum Asked {
    Question(String),
    Refused(String),
}

/// Reads the agent's events after `after` until Sculptured asks about the proposal or refuses it.
async fn asked(api: &Api, seat: &str, after: i64, message: &str) -> Result<Asked> {
    let query = [("after", after.to_string()), ("limit", "500".into())];
    poll::until("the proposal recorded", || async {
        let events = api.get(&["seats", seat, "events"], &query).await?;
        let events = events.expect("the agent's events")?;
        let all = events.as_array().map(Vec::as_slice).unwrap_or_default();
        let bodies: Vec<&Value> = all.iter().map(|e| &e["body"]).collect();
        Ok(resolve(&bodies, message))
    })
    .await
}

/// What the agent's events since the message say. The agent had no question open, so the first
/// approval question is about its proposal (Sculptured may record it before the turn, without a
/// turn id). A refusal is the reply of the turn that took `message`; an earlier turn may still be
/// ending meanwhile.
fn resolve(bodies: &[&Value], message: &str) -> Option<Asked> {
    let approval = |b: &&&Value| b["type"] == "question-raised" && b["source"] == "approval";
    if let Some(q) = bodies.iter().find(approval) {
        let id = q["prompt"]["tool_input"]["proposal_id"].as_str()?;
        return Some(Asked::Question(id.to_string()));
    }
    let took = |b: &&&Value| b["type"] == "message-consumed" && b["message_id"] == message;
    let turn = bodies.iter().find(took)?["turn_id"].as_str()?;
    let reply = |b: &&&Value| b["type"] == "agent-text" && b["turn_id"].as_str() == Some(turn);
    let text = bodies.iter().find(reply)?["text"]
        .as_str()
        .unwrap_or_default();
    Some(Asked::Refused(
        text.strip_prefix(FAILED).unwrap_or(text).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_proposal_is_asked_about_or_refused() {
        let earlier = json!({"type": "agent-text", "turn_id": "turn-1", "text": "Approved"});
        let asked = json!({"type": "question-raised", "source": "approval", "turn_id": null,
                           "prompt": {"tool_input": {"proposal_id": "chg-1"}}});
        let found = resolve(&[&earlier, &asked], "m2");
        assert!(matches!(found, Some(Asked::Question(id)) if id == "chg-1"));
        let took = json!({"type": "message-consumed", "message_id": "m2", "turn_id": "turn-2"});
        assert!(resolve(&[&earlier], "m2").is_none());
        assert!(
            resolve(&[&earlier, &took], "m2").is_none(),
            "an earlier turn's reply"
        );
        let failed = json!({"type": "agent-text", "turn_id": "turn-2",
                            "text": "Practice collaboration failed: payload must be an object"});
        let why = resolve(&[&earlier, &took, &failed], "m2");
        assert!(matches!(why, Some(Asked::Refused(w)) if w == "payload must be an object"));
    }
}
