//! The tools agents talk to each other with: `handoff`, `message` and `inbox`, and in a dispatch
//! run `delegate`, `report` and `finish` in place of `handoff` (see `dispatch`). They are arena's,
//! not the world's: optional, recorded when used, and never a condition for anything (except in
//! runs recorded before they were optional, where every agent but the last had to hand off, and
//! the lead's `finish`, which ends a dispatch run).

use serde_json::{json, Value};

use crate::model::*;

use super::{
    dispatch::{self, Said},
    tool_call_payload, BranchCtx, CallFacts, ToolExec,
};

pub const HANDOFF: &str = "handoff";
pub const MESSAGE: &str = "message";
pub const INBOX: &str = "inbox";

/// True when `name` is a talk tool on `branch` (a legacy run has only `handoff`).
pub fn is_talk(branch: &BranchCtx, name: &str) -> bool {
    name == HANDOFF
        || (!branch.rules.handoff_required && [MESSAGE, INBOX].contains(&name))
        || dispatch::is_dispatch_talk(branch, name)
}

/// `handoff` as runs recorded before it was optional offered it.
pub fn handoff_schema() -> Value {
    json!({"name": HANDOFF,
           "description": "Pass the conversation to the next agent with a note summarising what you learned and what remains. Call it exactly once, when you are done.",
           "inputSchema": {"type": "object",
                           "properties": {"note": {"type": "string"}},
                           "required": ["note"]}})
}

fn optional_handoff_schema() -> Value {
    json!({"name": HANDOFF,
           "description": "Optional. Leave a note for the agents that start after you (or for one agent, `to`): what you learned and what remains. It ends nothing; you may call it more than once.",
           "inputSchema": {"type": "object",
                           "properties": {"note": {"type": "string"}, "to": {"type": "string"}},
                           "required": ["note"]}})
}

fn message_schema() -> Value {
    json!({"name": MESSAGE,
           "description": "Optional. Send a message to another agent of your team by id, or to everyone with \"*\". They read it with `inbox`.",
           "inputSchema": {"type": "object",
                           "properties": {"to": {"type": "string"}, "text": {"type": "string"}},
                           "required": ["to", "text"]}})
}

fn inbox_schema() -> Value {
    json!({"name": INBOX,
           "description": "Read the messages and handoff notes other agents sent you so far, oldest first.",
           "inputSchema": {"type": "object", "properties": {}}})
}

/// The talk tools `agent` gets: all three, or in a legacy run `handoff` unless it is last; in a
/// dispatch run `message` and `inbox` with the dispatch tools (see `dispatch::schemas`).
pub fn schemas(branch: &BranchCtx, agent: &str) -> Vec<Value> {
    if branch.rules.runner == Runner::Dispatch {
        let mut tools = dispatch::schemas(branch, agent);
        tools.extend([message_schema(), inbox_schema()]);
        return tools;
    }
    if !branch.rules.handoff_required {
        return vec![optional_handoff_schema(), message_schema(), inbox_schema()];
    }
    if branch.is_last(agent) {
        return vec![];
    }
    vec![handoff_schema()]
}

fn text_arg(args: &Value, key: &str) -> String {
    let v = args.get(key).and_then(Value::as_str).unwrap_or("");
    v.trim().to_string()
}

/// `agent`'s handoff, or why it can't be taken.
fn take_handoff(branch: &BranchCtx, agent: &str, args: &Value) -> Result<HandoffPayload, String> {
    let note = text_arg(args, "note");
    let to = Some(text_arg(args, "to")).filter(|t| !t.is_empty());
    if branch.rules.handoff_required && branch.is_last(agent) {
        return Err("handoff is not available to the last agent".to_string());
    }
    if branch.rules.runner == Runner::Dispatch {
        return Err(dispatch::NO_HANDOFF.to_string());
    }
    if note.is_empty() {
        return Err("handoff() requires a non-empty 'note' string".to_string());
    }
    if let Some(t) = to.as_deref().filter(|t| !branch.has_agent(t)) {
        return Err(format!("there is no agent {t:?} in this team"));
    }
    let mut h = branch.handoffs.lock().unwrap();
    if branch.rules.handoff_required && h.contains_key(agent) {
        return Err("handoff already sent".to_string());
    }
    h.insert(agent.to_string(), note.clone());
    Ok(HandoffPayload { note, to })
}

/// `agent`'s message, or why it can't be sent.
fn take_message(branch: &BranchCtx, args: &Value) -> Result<MessagePayload, String> {
    let (to, text) = (text_arg(args, "to"), text_arg(args, "text"));
    if text.is_empty() {
        return Err("message() requires a non-empty 'text' string".to_string());
    }
    if to != EVERYONE && !branch.has_agent(&to) {
        return Err(format!("there is no agent {to:?} in this team"));
    }
    Ok(MessagePayload { to, text })
}

/// True when the handoff, message, delegation or report `e` is for `agent` (and not from it).
fn is_for(e: &Event, agent: &str) -> bool {
    let to = e.payload.get("to").and_then(Value::as_str);
    let mine = match e.kind {
        EventKind::Handoff => to.is_none_or(|t| t == agent),
        EventKind::Message => to.is_some_and(|t| t == agent || t == EVERYONE),
        EventKind::Delegation | EventKind::Report => to == Some(agent),
        _ => false,
    };
    mine && e.agent.as_deref() != Some(agent)
}

/// The text of a handoff (`note`), message or report (`text`), or delegation (`task`).
pub fn said_text(e: &Event) -> Option<&Value> {
    ["note", "text", "task"]
        .iter()
        .find_map(|k| e.payload.get(*k))
}

/// The handoffs, messages, delegations and reports in `events` for `agent`, oldest first, as
/// `{from, kind, to?, text}`.
pub fn mail_for(events: &[Event], agent: &str) -> Vec<Value> {
    let text = |e: &Event| said_text(e).cloned();
    events
        .iter()
        .filter(|e| is_for(e, agent))
        .map(|e| json!({"from": e.agent, "kind": e.kind, "to": e.payload.get("to"), "text": text(e)}))
        .collect()
}

impl ToolExec {
    /// Runs one of the talk tools for `agent` (behind the branch gate, which the caller holds).
    pub(super) async fn talk(
        &self,
        branch: &BranchCtx,
        agent: &str,
        name: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        let taken = match name {
            HANDOFF => take_handoff(branch, agent, &args).map(Taken::Handoff),
            MESSAGE => take_message(branch, &args).map(Taken::Message),
            INBOX => Ok(Taken::Inbox),
            _ => dispatch::take(branch, agent, name, &args).map(Taken::Dispatch),
        };
        match taken {
            Ok(Taken::Handoff(p)) => self.recorded(branch, agent, EventKind::Handoff, &p).await,
            Ok(Taken::Message(p)) => {
                let said = Said::new(agent, EventKind::Message, Some(&p.to), &p.text);
                self.told(branch, said, &p).await
            }
            Ok(Taken::Dispatch(d)) => self.dispatched(branch, agent, d).await,
            Ok(Taken::Inbox) => self.inbox(branch, agent, args, tool_call_id).await,
            Err(e) => {
                self.refused(branch, agent, name, args, e, tool_call_id)
                    .await
            }
        }
    }

    async fn recorded<T: serde::Serialize>(
        &self,
        branch: &BranchCtx,
        agent: &str,
        kind: EventKind,
        payload: &T,
    ) -> Result<Value, String> {
        self.record(branch, Some(agent), kind, payload).await?;
        Ok(json!({"ok": true}))
    }

    /// Records what an agent said (`said`, its payload `payload`) and tells the branch's dispatch
    /// runner, if it has one.
    pub(super) async fn told<T: serde::Serialize>(
        &self,
        branch: &BranchCtx,
        mut said: Said,
        payload: &T,
    ) -> Result<Value, String> {
        let agent = said.from.clone();
        said.seq = self
            .record_seq(branch, Some(&agent), said.kind, payload)
            .await?;
        branch.tell(said);
        Ok(json!({"ok": true}))
    }

    /// The mail for `agent` so far, recorded as a tool call that reached no part.
    async fn inbox(
        &self,
        branch: &BranchCtx,
        agent: &str,
        args: Value,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        let events = self.store.events(&branch.branch_id).await;
        let events = events.map_err(|e| format!("internal error: reading the record: {e:#}"))?;
        let outcome = Ok(json!({"messages": mail_for(&events, agent)}));
        let payload = tool_call_payload(INBOX, args, &outcome, tool_call_id, CallFacts::default());
        self.record(branch, Some(agent), EventKind::ToolCall, &payload)
            .await?;
        outcome
    }

    async fn refused(
        &self,
        branch: &BranchCtx,
        agent: &str,
        name: &str,
        args: Value,
        error: String,
        tool_call_id: Option<String>,
    ) -> Result<Value, String> {
        let outcome = Err(error);
        let payload = tool_call_payload(name, args, &outcome, tool_call_id, CallFacts::default());
        self.record(branch, Some(agent), EventKind::ToolCall, &payload)
            .await?;
        outcome
    }
}

enum Taken {
    Handoff(HandoffPayload),
    Message(MessagePayload),
    Dispatch(dispatch::Taken),
    Inbox,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(agent: &str, kind: EventKind, payload: Value) -> Event {
        crate::testkit::event(1, Some(agent), kind, payload)
    }

    #[test]
    fn mail_is_what_others_sent_to_the_agent_or_to_all() {
        let evs = vec![
            ev("a", EventKind::Handoff, json!({"note": "for whoever"})),
            ev("a", EventKind::Handoff, json!({"note": "for c", "to": "c"})),
            ev("a", EventKind::Message, json!({"to": "b", "text": "hi b"})),
            ev(
                "c",
                EventKind::Message,
                json!({"to": "*", "text": "hi all"}),
            ),
            ev("b", EventKind::Message, json!({"to": "*", "text": "mine"})),
            ev("a", EventKind::ToolCall, json!({"name": "get", "to": "b"})),
        ];
        let texts = |agent| -> Vec<Value> {
            mail_for(&evs, agent)
                .iter()
                .map(|m| m["text"].clone())
                .collect()
        };
        assert_eq!(
            texts("b"),
            [json!("for whoever"), json!("hi b"), json!("hi all")]
        );
        assert_eq!(
            texts("c"),
            [json!("for whoever"), json!("for c"), json!("mine")]
        );
        assert_eq!(texts("a"), [json!("hi all"), json!("mine")]);
    }
}
