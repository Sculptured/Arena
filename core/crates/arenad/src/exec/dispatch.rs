//! The talk tools of a dispatch run (see `crate::dispatch`): the lead's `delegate` and `finish`,
//! and the other agents' `report`. Each is recorded as its own event, and the branch's dispatch
//! runner is told, so it wakes whoever the work or the report is for.

use serde_json::{json, Value};

use crate::model::*;

use super::{BranchCtx, ToolExec};

pub const DELEGATE: &str = "delegate";
pub const REPORT: &str = "report";
pub const FINISH: &str = "finish";

/// Why `handoff` is refused in a dispatch run.
pub const NO_HANDOFF: &str =
    "a dispatch run has no handoff: report to the lead with `report`, or use `message`";

/// What a call is told once the lead has finished the run.
pub const CLOSED: &str = "the run is over: the lead finished it";

/// Something an agent said that a dispatch runner acts on: a delegation, report, message or the
/// lead's finish, recorded at `seq`.
#[derive(Debug, Clone, PartialEq)]
pub struct Said {
    pub seq: i64,
    pub from: String,
    pub kind: EventKind,
    /// The agent it is for (`*`: everyone); none for a finish.
    pub to: Option<String>,
    pub text: String,
}

impl Said {
    /// What `from` said (its seq set once it is recorded).
    pub fn new(from: &str, kind: EventKind, to: Option<&str>, text: &str) -> Said {
        Said {
            seq: 0,
            from: from.to_string(),
            kind,
            to: to.map(str::to_string),
            text: text.to_string(),
        }
    }
}

/// True when `name` is one of the dispatch tools and `branch` is a dispatch run.
pub fn is_dispatch_talk(branch: &BranchCtx, name: &str) -> bool {
    branch.rules.runner == Runner::Dispatch && [DELEGATE, REPORT, FINISH].contains(&name)
}

/// The dispatch tools `agent` gets: the lead `delegate` and `finish`, every other agent `report`.
pub fn schemas(branch: &BranchCtx, agent: &str) -> Vec<Value> {
    if branch.is_lead(agent) {
        return vec![delegate_schema(branch, agent), finish_schema()];
    }
    vec![report_schema()]
}

/// The team the lead may delegate to, as `id (name)`.
fn roster(branch: &BranchCtx, lead: &str) -> String {
    let others = branch.team.agents.iter().filter(|a| a.id != lead);
    let named: Vec<String> = others.map(|a| format!("`{}` ({})", a.id, a.name)).collect();
    named.join(", ")
}

fn delegate_schema(branch: &BranchCtx, lead: &str) -> Value {
    let description = format!(
        "Give one agent of your team work: `to` is its id, `task` what to do, with the facts and \
         ids it needs. It starts at once (or when it finishes what it is doing), works in \
         parallel with the others, and reports back to you. Your team: {}.",
        roster(branch, lead)
    );
    json!({"name": DELEGATE, "description": description,
           "inputSchema": {"type": "object",
                           "properties": {"to": {"type": "string"}, "task": {"type": "string"}},
                           "required": ["to", "task"]}})
}

fn finish_schema() -> Value {
    json!({"name": FINISH,
           "description": "End the run once the work is done and you have checked it: every agent is stopped and the world is scored. Give a short summary of what the team did. Nothing can be done after it.",
           "inputSchema": {"type": "object",
                           "properties": {"summary": {"type": "string"}},
                           "required": ["summary"]}})
}

fn report_schema() -> Value {
    json!({"name": REPORT,
           "description": "Report to the lead what you did and found, with the numbers and ids it needs. You then wait (you are not done) until the lead or another agent sends you more.",
           "inputSchema": {"type": "object",
                           "properties": {"text": {"type": "string"}},
                           "required": ["text"]}})
}

/// A dispatch tool call that may be recorded.
pub enum Taken {
    Delegation(DelegationPayload),
    Report(ReportPayload),
    Finish(FinishPayload),
}

fn text_arg(args: &Value, key: &str) -> String {
    let v = args.get(key).and_then(Value::as_str).unwrap_or("");
    v.trim().to_string()
}

fn required(args: &Value, tool: &str, key: &str) -> Result<String, String> {
    let v = text_arg(args, key);
    match v.is_empty() {
        true => Err(format!("{tool}() requires a non-empty '{key}' string")),
        false => Ok(v),
    }
}

/// `agent`'s call of the dispatch tool `name`, or why it is refused.
pub fn take(branch: &BranchCtx, agent: &str, name: &str, args: &Value) -> Result<Taken, String> {
    let lead = branch.lead_id();
    match (name, agent == lead) {
        (DELEGATE, true) => delegation(branch, agent, args).map(Taken::Delegation),
        (FINISH, true) => Ok(Taken::Finish(FinishPayload {
            summary: required(args, FINISH, "summary")?,
        })),
        (REPORT, false) => Ok(Taken::Report(ReportPayload {
            to: lead,
            text: required(args, REPORT, "text")?,
            turn_end: false,
        })),
        (REPORT, true) => Err("the lead reports to no one: use `finish` to end the run".into()),
        _ => Err(format!("only the lead ({lead}) may {name}")),
    }
}

fn delegation(branch: &BranchCtx, lead: &str, args: &Value) -> Result<DelegationPayload, String> {
    let to = required(args, DELEGATE, "to")?;
    let task = required(args, DELEGATE, "task")?;
    if to == lead {
        return Err("delegate to another agent of your team, not to yourself".into());
    }
    if !branch.team.agents.iter().any(|a| a.id == to) {
        let team = roster(branch, lead);
        return Err(format!("there is no agent {to:?} in this team: {team}"));
    }
    Ok(DelegationPayload { to, task })
}

impl ToolExec {
    /// Records a dispatch tool call and tells the runner; a finish also closes the branch, behind
    /// the gate the caller holds, so no call lands after it.
    pub(super) async fn dispatched(
        &self,
        branch: &BranchCtx,
        agent: &str,
        taken: Taken,
    ) -> Result<Value, String> {
        match taken {
            Taken::Delegation(p) => {
                let said = Said::new(agent, EventKind::Delegation, Some(&p.to), &p.task);
                self.told(branch, said, &p).await
            }
            Taken::Report(p) => {
                let said = Said::new(agent, EventKind::Report, Some(&p.to), &p.text);
                self.told(branch, said, &p).await
            }
            Taken::Finish(p) => {
                let said = Said::new(agent, EventKind::Finish, None, &p.summary);
                self.told(branch, said, &p).await?;
                branch.close();
                Ok(
                    json!({"ok": true, "note": "The run is finishing: every agent is stopped and the world is scored."}),
                )
            }
        }
    }
}
