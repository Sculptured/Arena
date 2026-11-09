//! A fork of a dispatch run, read from the record it inherits (the events before the fork point):
//! every agent restarts in a new session with its own history as the start of its first turn;
//! the agents that were working at the fork point carry on at once, mail sent but not yet
//! delivered is delivered, and the lead's idle wakes so far still count.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::board::{Mail, MailKind};
use crate::{exec::said_text, model::*};

#[derive(Debug, Clone, PartialEq)]
pub struct Resumed {
    pub idle_wakes: u32,
    /// Mail recorded for an agent before the fork point that no turn of it had taken.
    pub pending: Vec<(String, Mail)>,
    /// The agents working at the fork point (woken, their turn not ended).
    pub working: Vec<String>,
    /// Each agent's history before the fork point, as text.
    pub history: HashMap<String, String>,
    /// Agents whose start and end are both in the inherited record: they start again.
    pub restart: Vec<String>,
    /// Some agent was woken before the fork point (else the lead starts as in a new run).
    pub woke: bool,
}

/// The most of a tool's result or error a history quotes.
const RESULT_CHARS: usize = 1200;

fn cut(text: &str) -> String {
    match text.char_indices().nth(RESULT_CHARS) {
        Some((i, _)) => format!("{}… (cut)", &text[..i]),
        None => text.to_string(),
    }
}

fn str_of<'a>(e: &'a Event, key: &str) -> &'a str {
    e.payload.get(key).and_then(Value::as_str).unwrap_or("")
}

/// `None` when the record has no agent start yet: the run begins afresh.
pub fn resumed(events: &[Event], team: &Team) -> Option<Resumed> {
    if !events.iter().any(|e| e.kind == EventKind::AgentStart) {
        return None;
    }
    let ids: Vec<&str> = team.agents.iter().map(|a| a.id.as_str()).collect();
    Some(Resumed {
        idle_wakes: idle_wakes(events),
        pending: pending(events, &ids),
        working: ids
            .iter()
            .filter(|id| working(events, id))
            .map(|id| id.to_string())
            .collect(),
        history: ids
            .iter()
            .map(|id| (id.to_string(), history(events, id)))
            .collect(),
        restart: ids
            .iter()
            .filter(|id| balanced(events, id))
            .map(|id| id.to_string())
            .collect(),
        woke: events.iter().any(|e| e.kind == EventKind::Wake),
    })
}

fn idle_wakes(events: &[Event]) -> u32 {
    let idle = events
        .iter()
        .filter(|e| e.kind == EventKind::Wake && str_of(e, "reason") == "idle");
    idle.count() as u32
}

/// True when `agent`'s last wake has no idle after it.
fn working(events: &[Event], agent: &str) -> bool {
    let last = events.iter().rev().find(|e| {
        e.agent.as_deref() == Some(agent) && matches!(e.kind, EventKind::Wake | EventKind::Idle)
    });
    last.is_some_and(|e| e.kind == EventKind::Wake)
}

fn balanced(events: &[Event], agent: &str) -> bool {
    let count = |k: EventKind| {
        let mine = events
            .iter()
            .filter(|e| e.kind == k && e.agent.as_deref() == Some(agent));
        mine.count()
    };
    count(EventKind::AgentStart) == count(EventKind::AgentEnd)
}

fn mail_kind(kind: EventKind) -> Option<MailKind> {
    match kind {
        EventKind::Delegation => Some(MailKind::Delegation),
        EventKind::Report => Some(MailKind::Report),
        EventKind::Message => Some(MailKind::Message),
        _ => None,
    }
}

/// Who mail `e` went to: its `to`, or everyone but its sender for `*`.
fn recipients(e: &Event, ids: &[&str]) -> Vec<String> {
    let from = e.agent.as_deref().unwrap_or("");
    match str_of(e, "to") {
        "*" => ids
            .iter()
            .filter(|id| **id != from)
            .map(|id| id.to_string())
            .collect(),
        to if ids.contains(&to) && to != from => vec![to.to_string()],
        _ => vec![],
    }
}

/// The seqs of the mail each agent's wakes took.
fn delivered(events: &[Event]) -> HashSet<(String, i64)> {
    let wakes = events.iter().filter(|e| e.kind == EventKind::Wake);
    let each = wakes.flat_map(|e| {
        let agent = e.agent.clone().unwrap_or_default();
        let seqs = e.payload["mail"].as_array().cloned().unwrap_or_default();
        seqs.into_iter()
            .filter_map(move |s| Some((agent.clone(), s.as_i64()?)))
    });
    each.collect()
}

fn pending(events: &[Event], ids: &[&str]) -> Vec<(String, Mail)> {
    let taken = delivered(events);
    let mut out = vec![];
    for e in events {
        let Some(kind) = mail_kind(e.kind) else {
            continue;
        };
        let text = said_text(e)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        for to in recipients(e, ids) {
            if !taken.contains(&(to.clone(), e.seq)) {
                let from = e.agent.clone().unwrap_or_default();
                let mail = Mail {
                    seq: e.seq,
                    from,
                    kind,
                    text: text.clone(),
                };
                out.push((to, mail));
            }
        }
    }
    out
}

/// A tool call in a history: its name, arguments and (cut) result or error.
fn call_line(e: &Event) -> String {
    let p = &e.payload;
    let out = match p.get("error").and_then(Value::as_str) {
        Some(err) => format!("error: {err}"),
        None => p.get("result").map_or(String::new(), Value::to_string),
    };
    let name = str_of(e, "name");
    format!("- You called `{name}` {} → {}", p["args"], cut(&out))
}

/// One line of `agent`'s history for event `e`, if `e` is its own.
fn line(e: &Event) -> Option<String> {
    let p = &e.payload;
    Some(match e.kind {
        EventKind::Wake if str_of(e, "reason") == "start" => "## Your first turn began".into(),
        EventKind::Wake => format!(
            "## You were woken ({}) with:\n{}",
            str_of(e, "reason"),
            str_of(e, "input").trim()
        ),
        EventKind::ToolCall => call_line(e),
        EventKind::Delegation => format!(
            "- You gave `{}` work: {}",
            str_of(e, "to"),
            str_of(e, "task")
        ),
        EventKind::Report if p["turn_end"] == true => format!(
            "- Your turn's last words went to the lead: {}",
            str_of(e, "text")
        ),
        EventKind::Report => format!("- You reported to the lead: {}", str_of(e, "text")),
        EventKind::Message => format!(
            "- You messaged `{}`: {}",
            str_of(e, "to"),
            str_of(e, "text")
        ),
        EventKind::Idle if p["ok"] == false => {
            format!("- Your turn failed: {}", str_of(e, "error"))
        }
        EventKind::Idle => format!("- You ended your turn saying: {}", str_of(e, "text")),
        _ => return None,
    })
}

fn history(events: &[Event], agent: &str) -> String {
    let mine = events.iter().filter(|e| e.agent.as_deref() == Some(agent));
    let lines: Vec<String> = mine.filter_map(line).collect();
    match lines.is_empty() {
        true => "You had not worked yet.".into(),
        false => lines.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(seq: i64, agent: &str, kind: EventKind, payload: Value) -> Event {
        Event {
            branch_id: "b".into(),
            seq,
            at: chrono::Utc::now(),
            agent: Some(agent.into()).filter(|a: &String| !a.is_empty()),
            kind,
            payload,
        }
    }

    fn team() -> Team {
        let spec = |id: &str| AgentSpec {
            tools: Vec::new(),
            id: id.into(),
            name: id.into(),
            kind: AgentKind::ClaudeCode,
            model: None,
            prompt: String::new(),
            provider: None,
            read_only: false,
            lead: false,
            url: None,
            headers: Default::default(),
        };
        Team {
            id: "t".into(),
            name: "T".into(),
            agents: vec![spec("lead"), spec("qa"), spec("supply")],
        }
    }

    /// The lead delegated to both; qa took its work and reported; supply's work came while it was
    /// busy with something else; the lead was woken once for an idle team.
    fn record() -> Vec<Event> {
        [starts(), delegations(), work(), reports(), idling()].concat()
    }

    fn starts() -> Vec<Event> {
        vec![
            ev(1, "", EventKind::RunStart, json!({})),
            ev(2, "lead", EventKind::AgentStart, json!({})),
            ev(3, "qa", EventKind::AgentStart, json!({})),
            ev(4, "supply", EventKind::AgentStart, json!({})),
            ev(
                5,
                "lead",
                EventKind::Wake,
                json!({"reason": "start", "mail": [], "input": "ticket"}),
            ),
        ]
    }

    fn delegations() -> Vec<Event> {
        vec![
            ev(
                6,
                "lead",
                EventKind::Delegation,
                json!({"to": "qa", "task": "hold CVX-2611"}),
            ),
            ev(
                7,
                "lead",
                EventKind::Message,
                json!({"to": "supply", "text": "warm up"}),
            ),
        ]
    }

    fn work() -> Vec<Event> {
        vec![
            ev(
                8,
                "qa",
                EventKind::Wake,
                json!({"reason": "mail", "mail": [6], "input": "# Work\nhold CVX-2611"}),
            ),
            ev(
                9,
                "supply",
                EventKind::Wake,
                json!({"reason": "mail", "mail": [7], "input": "warm up"}),
            ),
            ev(
                10,
                "qa",
                EventKind::ToolCall,
                json!({"name": "propose_change", "args": {"action": "batch_hold"}, "result": {"id": "C-1"}}),
            ),
        ]
    }

    fn reports() -> Vec<Event> {
        vec![
            ev(
                11,
                "lead",
                EventKind::Delegation,
                json!({"to": "supply", "task": "allocate"}),
            ),
            ev(
                12,
                "qa",
                EventKind::Report,
                json!({"to": "lead", "text": "hold proposed: C-1"}),
            ),
        ]
    }

    fn idling() -> Vec<Event> {
        vec![
            ev(
                13,
                "qa",
                EventKind::Idle,
                json!({"ok": true, "text": "done"}),
            ),
            ev(
                14,
                "lead",
                EventKind::Idle,
                json!({"ok": true, "text": "waiting"}),
            ),
            ev(
                15,
                "lead",
                EventKind::Wake,
                json!({"reason": "idle", "mail": [], "idle_wake": 1, "input": "team waiting"}),
            ),
        ]
    }

    #[test]
    fn a_fresh_run_has_nothing_to_resume() {
        assert_eq!(resumed(&record()[..1], &team()), None);
    }

    #[test]
    fn a_fork_knows_who_worked_what_waited_and_the_idle_wakes() {
        let r = resumed(&record(), &team()).unwrap();
        assert_eq!(r.idle_wakes, 1);
        assert_eq!(r.working, ["lead", "supply"]);
        let pending: Vec<(&str, i64)> = r
            .pending
            .iter()
            .map(|(to, m)| (to.as_str(), m.seq))
            .collect();
        assert_eq!(
            pending,
            [("supply", 11), ("lead", 12)],
            "supply's second task and the report"
        );
        assert_eq!(r.pending[0].1.kind, MailKind::Delegation);
        assert!(r.restart.is_empty(), "every start is still open");
    }

    #[test]
    fn each_agent_gets_its_own_history() {
        let r = resumed(&record(), &team()).unwrap();
        let qa = &r.history["qa"];
        assert!(
            qa.contains("You were woken (mail) with:\n# Work\nhold CVX-2611"),
            "{qa}"
        );
        assert!(
            qa.contains(
                "You called `propose_change` {\"action\":\"batch_hold\"} → {\"id\":\"C-1\"}"
            ),
            "{qa}"
        );
        assert!(qa.contains("You reported to the lead: hold proposed: C-1"));
        assert!(qa.contains("You ended your turn saying: done"));
        assert!(!qa.contains("allocate"), "supply's work is not qa's");
        let lead = &r.history["lead"];
        assert!(lead.starts_with("## Your first turn began"));
        assert!(lead.contains("You gave `qa` work: hold CVX-2611"));
    }

    #[test]
    fn an_agent_whose_end_is_inherited_starts_again() {
        let mut evs = record();
        evs.push(ev(16, "qa", EventKind::AgentEnd, json!({"ok": true})));
        let r = resumed(&evs, &team()).unwrap();
        assert_eq!(r.restart, ["qa"]);
    }
}
