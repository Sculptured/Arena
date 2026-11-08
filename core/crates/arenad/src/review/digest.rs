//! A run's digest for the reviewer: what a person would look at to see where a run went wrong,
//! built from the record alone, deterministic, and bounded in size so a review stays cheap.
//!
//! Every item carries its step (`seq`), so the reviewer can cite steps and arenad can check each
//! citation against the branch.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;

use super::text::{cut, cut_json, one_line};
use crate::{
    metrics::{metrics, Metrics},
    model::*,
    prices,
};

/// The bound on a run's digest, as compact JSON. Long lists are capped and summarised until the
/// digest fits ([`LEVELS`]).
pub const MAX_DIGEST_BYTES: usize = 40_000;

/// What the digest is built from: the branch, its events, and its task's ticket when known.
pub struct RunRecord<'a> {
    pub branch: &'a Branch,
    pub events: &'a [Event],
    pub ticket: Option<&'a str>,
}

/// A list cut to a cap: the items kept, and how many were left out.
#[derive(Debug, Clone, Serialize)]
pub struct Capped<T> {
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl<T> Capped<T> {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.omitted == 0
    }

    /// The first `cap` of `all`.
    pub(crate) fn first(mut all: Vec<T>, cap: usize) -> Capped<T> {
        let omitted = all.len().saturating_sub(cap);
        all.truncate(cap);
        Capped {
            items: all,
            omitted,
        }
    }

    /// The first two thirds and the last third of `cap` items of `all`: how a list starts and
    /// how it ends both matter.
    fn ends(mut all: Vec<T>, cap: usize) -> Capped<T> {
        if all.len() <= cap {
            return Capped::first(all, cap);
        }
        let tail = cap / 3;
        let omitted = all.len() - cap;
        let last = all.split_off(all.len() - tail);
        all.truncate(cap - tail);
        all.extend(last);
        Capped {
            items: all,
            omitted,
        }
    }
}

/// How much of each list and text the digest keeps; tighter levels follow until it fits.
#[derive(Debug, Clone, Copy)]
pub struct Caps {
    pub problems: usize,
    pub writes: usize,
    pub notes: usize,
    pub world: usize,
    pub checks: usize,
    pub agents: usize,
    /// Bytes of a long text (a ticket, a handoff note, an agent's prompt).
    pub long: usize,
    /// Bytes of a short text (a call's arguments or result, an error).
    pub short: usize,
}

/// The levels tried in turn, the first whose digest fits [`MAX_DIGEST_BYTES`] kept.
pub const LEVELS: [Caps; 3] = [
    Caps {
        problems: 25,
        writes: 45,
        notes: 14,
        world: 30,
        checks: 40,
        agents: 12,
        long: 900,
        short: 260,
    },
    Caps {
        problems: 15,
        writes: 25,
        notes: 10,
        world: 15,
        checks: 30,
        agents: 10,
        long: 500,
        short: 160,
    },
    Caps {
        problems: 8,
        writes: 12,
        notes: 6,
        world: 8,
        checks: 25,
        agents: 8,
        long: 260,
        short: 100,
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct Digest {
    pub branch: BranchLine,
    pub task: TaskLine,
    pub team: TeamLine,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fork: Option<ForkLine>,
    pub steps: StepRange,
    /// The scoring at the end; absent when the branch ended unscored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checks: Option<Checks>,
    /// Refused and failed tool calls, with their errors.
    pub problems: Capped<Problem>,
    pub agents: Vec<AgentLine>,
    /// Handoff notes and messages between agents.
    pub notes: Capped<Note>,
    /// The disruptions the run's scenario added (as recorded in `run_start`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disruptions: Option<Value>,
    /// What the world's parts reported as their clocks ran (approvals, alarms, disruptions).
    pub world_events: Capped<WorldLine>,
    /// The agents' writes to the world that went through, arguments trimmed.
    pub writes: Writes,
    pub metrics: MetricsLine,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchLine {
    pub id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskLine {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TeamLine {
    pub name: String,
    /// `chain`, `concurrent` or `external` (a chain with handoffs required for old runs).
    pub runner: String,
    pub agents: Capped<AgentSpecLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentSpecLine {
    pub id: String,
    pub name: String,
    pub kind: AgentKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForkLine {
    pub parent: Option<String>,
    /// The first step of this branch's own; the steps before it are its parent's.
    pub start_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub changes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepRange {
    pub first: i64,
    pub last: i64,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Checks {
    pub seq: i64,
    pub passed: bool,
    pub items: Capped<CheckLine>,
    /// World rows that ended different from the expected result (gold scoring; empty
    /// with named checks).
    #[serde(skip_serializing_if = "Capped::is_empty")]
    pub differing: Capped<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unchecked: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckLine {
    pub name: String,
    pub passed: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub safety: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemKind {
    /// Held back before it ran: a gate, a read-only agent's write, a branch that can't go on.
    Refused,
    /// It ran and failed.
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub tool: String,
    pub kind: ProblemKind,
    pub error: String,
    pub args: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentLine {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// The read tools it called, and how often.
    pub reads: BTreeMap<String, usize>,
    pub writes: usize,
    pub problems: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// `handoff` or `message`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldLine {
    pub seq: i64,
    pub part: String,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Writes {
    /// How many of the agents' writes of each tool went through.
    pub by_tool: BTreeMap<String, usize>,
    /// Writes the world made itself (its links, such as an approval system deciding, and a
    /// task's setup), by tool: not the agents'.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub by_world: BTreeMap<String, usize>,
    #[serde(flatten)]
    pub list: Capped<Write>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Write {
    pub seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    pub tool: String,
    pub args: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsLine {
    pub seconds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_s: Option<f64>,
    pub tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    pub cost_label: String,
    pub agents: Vec<AgentUseLine>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentUseLine {
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub tokens: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The digest of a branch, at the first level that fits `max_bytes` (the tightest one when none
/// does).
pub fn digest(rec: &RunRecord, max_bytes: usize) -> Digest {
    let mut last = None;
    for caps in LEVELS {
        let d = build(rec, caps);
        if size_of(&d) <= max_bytes {
            return d;
        }
        last = Some(d);
    }
    last.expect("LEVELS is not empty")
}

/// The bytes of `d` as compact JSON.
pub fn size_of(d: &impl Serialize) -> usize {
    serde_json::to_vec(d).map_or(usize::MAX, |v| v.len())
}

/// The digest with these caps.
pub fn build(rec: &RunRecord, caps: Caps) -> Digest {
    let events = rec.events;
    let tools = Tools::of(events);
    Digest {
        branch: branch_line(rec.branch, caps),
        task: TaskLine {
            id: task_of(events),
            ticket: rec.ticket.map(|t| cut(t.trim(), caps.long * 2)),
        },
        team: team_line(rec.branch, events, caps),
        fork: fork_line(rec.branch, caps),
        steps: step_range(events),
        checks: checks(events, caps),
        problems: Capped::first(problems(events, caps), caps.problems),
        agents: agent_lines(rec.branch, events, &tools, caps),
        notes: Capped::ends(notes(events, caps), caps.notes),
        disruptions: run_start(events).and_then(|p| p.get("scenario").cloned()),
        world_events: world_lines(events, caps),
        writes: writes(events, &tools, caps),
        metrics: metrics_line(rec.branch, events),
    }
}

fn run_start(events: &[Event]) -> Option<&Value> {
    let rs = events.iter().find(|e| e.kind == EventKind::RunStart);
    rs.map(|e| &e.payload)
}

fn task_of(events: &[Event]) -> String {
    let id = run_start(events).and_then(|p| p["task_id"].as_str());
    id.unwrap_or("").to_string()
}

fn branch_line(b: &Branch, caps: Caps) -> BranchLine {
    BranchLine {
        id: b.id.clone(),
        status: b.status.as_str().to_string(),
        error: b.error.as_deref().map(|e| one_line(e, caps.long)),
    }
}

fn team_line(b: &Branch, events: &[Event], caps: Caps) -> TeamLine {
    let agent = |a: &AgentSpec| AgentSpecLine {
        id: a.id.clone(),
        name: a.name.clone(),
        kind: a.kind,
        model: a.model.clone(),
        provider: a.provider.clone(),
        read_only: a.read_only,
        prompt: one_line(&a.prompt, caps.long),
    };
    let rules = crate::recorded::rules(events);
    let runner = serde_json::to_value(rules.runner).ok();
    let mut runner = runner.and_then(|v| v.as_str().map(String::from));
    if rules.handoff_required {
        runner = runner.map(|r| format!("{r} (every agent but the last must hand off)"));
    }
    TeamLine {
        name: b.team.name.clone(),
        runner: runner.unwrap_or_default(),
        agents: Capped::first(b.team.agents.iter().map(agent).collect(), caps.agents),
    }
}

fn fork_line(b: &Branch, caps: Caps) -> Option<ForkLine> {
    b.parent_id.as_ref()?;
    Some(ForkLine {
        parent: b.parent_id.clone(),
        start_seq: b.start_seq,
        note: b.fork_note.as_deref().map(|n| one_line(n, caps.long)),
        changes: cut_json(&b.changes, caps.long),
    })
}

fn step_range(events: &[Event]) -> StepRange {
    StepRange {
        first: events.first().map_or(0, |e| e.seq),
        last: events.last().map_or(0, |e| e.seq),
        count: events.len(),
    }
}

fn checks(events: &[Event], caps: Caps) -> Option<Checks> {
    let ev = events.iter().rev().find(|e| e.kind == EventKind::Check)?;
    let p: CheckPayload = serde_json::from_value(ev.payload.clone()).ok()?;
    let line = |c: &parts::proto::NamedCheck| CheckLine {
        name: c.name.clone(),
        passed: c.passed,
        safety: c.safety,
        detail: (!c.detail.is_empty()).then(|| one_line(&c.detail, caps.long)),
    };
    let items = p.checks.iter().flatten().map(line).collect();
    Some(Checks {
        seq: ev.seq,
        passed: p.passed,
        items: Capped::first(items, caps.checks),
        // With named checks, `differing_rows` only repeats the failed checks' names.
        differing: match p.checks {
            Some(_) => Capped::first(vec![], 0),
            None => Capped::first(p.differing_rows.unwrap_or_default(), caps.checks),
        },
        unchecked: p.unchecked,
    })
}

/// Each tool's effect, as `run_start` records it (`{name: {effect, part?}}`).
struct Tools(BTreeSet<String>);

impl Tools {
    fn of(events: &[Event]) -> Tools {
        let map = run_start(events).and_then(|p| p["tools"].as_object());
        let writes = map.into_iter().flatten();
        let writes = writes.filter(|(_, t)| t["effect"] == "write");
        Tools(writes.map(|(name, _)| name.clone()).collect())
    }

    /// A call that writes: its tool is a write tool, or it wrote keys (runs recorded before
    /// tools carried their effect).
    fn writes(&self, p: &Value) -> bool {
        let named = p["name"].as_str().is_some_and(|n| self.0.contains(n));
        named || p["writes"].as_array().is_some_and(|w| !w.is_empty())
    }
}

fn tool_calls(events: &[Event]) -> impl Iterator<Item = &Event> {
    events.iter().filter(|e| e.kind == EventKind::ToolCall)
}

fn error_of(p: &Value) -> Option<&str> {
    p["error"].as_str().filter(|e| !e.is_empty())
}

/// A refusal: a gate's (`<tool> refused: …`), a read-only agent's write, or a stopped branch's.
fn kind_of(error: &str) -> ProblemKind {
    let refused = error.contains(" refused: ")
        || error.starts_with("Write tools are not available")
        || error.contains("read-only");
    match refused {
        true => ProblemKind::Refused,
        false => ProblemKind::Failed,
    }
}

fn problems(events: &[Event], caps: Caps) -> Vec<Problem> {
    let problem = |e: &Event| {
        let error = error_of(&e.payload)?;
        Some(Problem {
            seq: e.seq,
            agent: e.agent.clone(),
            tool: e.payload["name"].as_str().unwrap_or("").to_string(),
            kind: kind_of(error),
            error: one_line(error, caps.short * 2),
            args: cut_json(&e.payload["args"], caps.short),
        })
    };
    tool_calls(events).filter_map(problem).collect()
}

fn agent_lines(b: &Branch, events: &[Event], tools: &Tools, caps: Caps) -> Vec<AgentLine> {
    let ids = b.team.agents.iter().map(|a| a.id.as_str());
    let line = |id: &str| agent_line(id, events, tools, caps);
    ids.take(caps.agents).map(line).collect()
}

fn agent_line(id: &str, events: &[Event], tools: &Tools, caps: Caps) -> AgentLine {
    let own: Vec<&Event> = events
        .iter()
        .filter(|e| e.agent.as_deref() == Some(id))
        .collect();
    let first = |k: EventKind| own.iter().find(|e| e.kind == k).copied();
    let end = own.iter().rev().find(|e| e.kind == EventKind::AgentEnd);
    let end_p = end.and_then(|e| serde_json::from_value::<AgentEndPayload>(e.payload.clone()).ok());
    let calls = Calls::count(&own, tools);
    AgentLine {
        agent: id.to_string(),
        started: first(EventKind::AgentStart).map(|e| e.seq),
        ended: end.map(|e| e.seq),
        ok: end_p.as_ref().map(|p| p.ok),
        error: end_p
            .as_ref()
            .and_then(|p| p.error.as_deref())
            .map(|e| one_line(e, caps.long)),
        seconds: end_p.as_ref().and_then(|p| p.seconds).map(round1),
        reads: calls.reads,
        writes: calls.writes,
        problems: calls.problems,
    }
}

/// An agent's tool calls counted: its reads by tool, its writes, and its calls that failed.
struct Calls {
    reads: BTreeMap<String, usize>,
    writes: usize,
    problems: usize,
}

impl Calls {
    fn count(own: &[&Event], tools: &Tools) -> Calls {
        let mut c = Calls {
            reads: BTreeMap::new(),
            writes: 0,
            problems: 0,
        };
        for e in own.iter().filter(|e| e.kind == EventKind::ToolCall) {
            if error_of(&e.payload).is_some() {
                c.problems += 1;
            } else if tools.writes(&e.payload) {
                c.writes += 1;
            } else {
                let name = e.payload["name"].as_str().unwrap_or("").to_string();
                *c.reads.entry(name).or_insert(0) += 1;
            }
        }
        c
    }
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn notes(events: &[Event], caps: Caps) -> Vec<Note> {
    let note = |e: &Event| {
        let kind = match e.kind {
            EventKind::Handoff => "handoff",
            EventKind::Message => "message",
            _ => return None,
        };
        let text = e.payload["note"].as_str().or(e.payload["text"].as_str());
        Some(Note {
            seq: e.seq,
            agent: e.agent.clone(),
            kind,
            to: e.payload["to"].as_str().map(String::from),
            text: one_line(text.unwrap_or(""), caps.long),
        })
    };
    events.iter().filter_map(note).collect()
}

/// A world event that reads as something going wrong (a refusal, a failure, a disruption).
fn alarming(text: &str, kind: &str) -> bool {
    let t = format!("{kind} {text}").to_lowercase();
    [
        "reject", "fail", "disrupt", "error", "alarm", "breach", "refus", "late", "short",
    ]
    .iter()
    .any(|w| t.contains(w))
}

/// The world's events, the alarming ones first when there are too many, in step order.
fn world_lines(events: &[Event], caps: Caps) -> Capped<WorldLine> {
    let line = |e: &Event| WorldLine {
        seq: e.seq,
        part: e.payload["part"].as_str().unwrap_or("").to_string(),
        kind: e.payload["kind"].as_str().unwrap_or("").to_string(),
        text: one_line(e.payload["text"].as_str().unwrap_or(""), caps.short),
    };
    let all: Vec<WorldLine> = (events.iter())
        .filter(|e| e.kind == EventKind::WorldEvent)
        .map(line)
        .collect();
    let total = all.len();
    let (mut kept, rest): (Vec<_>, Vec<_>) =
        all.into_iter().partition(|w| alarming(&w.text, &w.kind));
    kept.truncate(caps.world);
    let room = caps.world - kept.len();
    kept.extend(rest.into_iter().take(room));
    kept.sort_by_key(|w| w.seq);
    let omitted = total - kept.len();
    Capped {
        items: kept,
        omitted,
    }
}

/// A step the world made itself: its links' or a task's setup's.
fn is_world_actor(agent: Option<&str>) -> bool {
    matches!(
        agent,
        Some(crate::exec::LINK_ACTOR | crate::exec::SETUP_ACTOR)
    )
}

fn writes(events: &[Event], tools: &Tools, caps: Caps) -> Writes {
    let ok_writes =
        tool_calls(events).filter(|e| error_of(&e.payload).is_none() && tools.writes(&e.payload));
    let (mut by_tool, mut by_world) = (BTreeMap::new(), BTreeMap::new());
    let mut all = vec![];
    for e in ok_writes {
        let tool = e.payload["name"].as_str().unwrap_or("").to_string();
        if is_world_actor(e.agent.as_deref()) {
            *by_world.entry(tool).or_insert(0) += 1;
            continue;
        }
        *by_tool.entry(tool.clone()).or_insert(0) += 1;
        all.push(Write {
            seq: e.seq,
            agent: e.agent.clone(),
            tool,
            args: cut_json(&e.payload["args"], caps.short),
            result: cut_json(&e.payload["result"], caps.short / 2),
        });
    }
    Writes {
        by_tool,
        by_world,
        list: Capped::ends(all, caps.writes),
    }
}

fn metrics_line(b: &Branch, events: &[Event]) -> MetricsLine {
    let now = b.ended_at.unwrap_or(b.created_at);
    let m: Metrics = metrics(b, events, prices::table(), now);
    let agent = |a: &crate::metrics::AgentUse| AgentUseLine {
        agent: a.agent.clone(),
        model: a.model.clone(),
        tokens: a.tokens,
        cost_usd: a.cost.usd.map(round4),
    };
    MetricsLine {
        seconds: round1(m.measured_s),
        estimated_s: m.estimated_s.map(round1),
        tokens: m.tokens,
        cost_usd: m.cost.usd.map(round4),
        cost_label: m.cost.label.clone(),
        agents: m.agents.iter().map(agent).collect(),
    }
}

fn round4(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Every step of the branch: the only seqs a review may cite.
pub fn steps_of(events: &[Event]) -> BTreeSet<i64> {
    events.iter().map(|e| e.seq).collect()
}

#[cfg(test)]
#[path = "digest_tests.rs"]
pub(crate) mod tests;
