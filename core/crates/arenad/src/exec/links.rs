//! Reporting a world's events: recording them and making the calls the world's links ask for. The
//! calls are recorded as ordinary tool calls of the actor `link`, so a replay repeats them from the
//! record and checks their results. All of a report's calls are made and saved before any is
//! recorded, and the saves go on the last one, so no step shows one of them without the others.

use std::collections::VecDeque;

use parts::proto::Link;
use serde_json::{json, Value};

use crate::{
    model::EventKind,
    overhead::{self, Cat},
    world::{link_args, PartEvent},
};

use super::{
    failure, outcome_of, save_step, tool_call_payload, world_event_payload, BranchCtx, CallFacts,
    ToolExec,
};

/// The actor of the steps links make; no agent may have this id.
pub const LINK_ACTOR: &str = "link";

/// The most calls one report may make, so links that set each other off can't run forever.
const MAX_LINK_CALLS: usize = 64;

/// A call the world makes of a part (for a link or a task's setup), made and not yet recorded.
pub(super) struct LinkStep {
    pub(super) tool: String,
    pub(super) args: Value,
    pub(super) outcome: Result<Value, String>,
    pub(super) facts: CallFacts,
}

enum Reported {
    Event(PartEvent),
    Link(LinkStep),
}

/// A link's step that didn't reach the part: no `part`, so a replay doesn't repeat it.
fn step_not_made(tool: &str, args: Value, error: String, clock_ms: i64) -> LinkStep {
    LinkStep {
        tool: tool.to_string(),
        args,
        outcome: Err(error),
        facts: CallFacts {
            clock_ms: Some(clock_ms),
            ..CallFacts::default()
        },
    }
}

/// Makes `link`'s call for the event with `data`.
async fn call_link(branch: &BranchCtx, link: &Link, data: &Value, clock_ms: i64) -> LinkStep {
    match link_args(link, data) {
        Ok(args) => call_part(branch, &link.to, &link.tool, args, clock_ms).await,
        Err(e) => step_not_made(&link.tool, json!({}), e, clock_ms),
    }
}

/// Calls `tool` of `part` with `args` for the world itself, marking the part written when the
/// call wrote, so the next save of what changed includes it.
pub(super) async fn call_part(
    branch: &BranchCtx,
    part: &str,
    tool: &str,
    args: Value,
    clock_ms: i64,
) -> LinkStep {
    let world = &branch.world;
    let output = match world
        .call_part_tool(part, tool, args.clone(), clock_ms)
        .await
    {
        Ok(output) => output,
        Err(e) => return step_not_made(tool, args, failure(branch, e), clock_ms),
    };
    let outcome = outcome_of(&output);
    let facts = CallFacts::of_output(Some(part.to_string()), output, clock_ms);
    if facts.changed_part() {
        world.mark_written(part);
    }
    LinkStep {
        tool: tool.to_string(),
        args,
        outcome,
        facts,
    }
}

/// `events` and every call they set off (a call's own events can set off more), in order.
async fn run_links(branch: &BranchCtx, events: Vec<PartEvent>) -> Vec<Reported> {
    let mut queue = VecDeque::from(events);
    let (mut log, mut calls) = (vec![], 0);
    while let Some(e) = queue.pop_front() {
        let links = branch.world.links_from(&e.part, &e.event.kind);
        let (data, clock_ms) = (e.event.data.clone(), e.clock_ms);
        log.push(Reported::Event(e));
        for link in &links {
            if calls == MAX_LINK_CALLS {
                let error = format!(
                    "links made {MAX_LINK_CALLS} calls in one step; this one and later ones were not made"
                );
                log.push(Reported::Link(step_not_made(
                    &link.tool,
                    json!({}),
                    error,
                    clock_ms,
                )));
                return log;
            }
            calls += 1;
            let mut step = call_link(branch, link, &data, clock_ms).await;
            queue.extend(std::mem::take(&mut step.facts.events));
            log.push(Reported::Link(step));
        }
    }
    log
}

/// Saves what the calls changed and puts the saves on the last call.
async fn save_on_last(branch: &BranchCtx, log: &mut [Reported]) {
    let last = log.iter_mut().rev().find_map(|r| match r {
        Reported::Link(step) => Some(step),
        Reported::Event(_) => None,
    });
    if let Some(step) = last {
        save_step(branch, None, &mut step.facts).await;
    }
}

impl ToolExec {
    /// Records `events` and makes the calls the world's links ask for at their time, each a step
    /// of its own. The seq of the last of those steps that wrote, if any did.
    pub(super) async fn report(
        &self,
        branch: &BranchCtx,
        events: Vec<PartEvent>,
    ) -> Result<Option<i64>, String> {
        let since = std::time::Instant::now();
        let mut log = run_links(branch, events).await;
        if log.iter().any(|r| matches!(r, Reported::Link(_))) {
            overhead::add_since(&branch.branch_id, Cat::Link, since);
        }
        save_on_last(branch, &mut log).await;
        let mut wrote = None;
        for entry in log {
            let seq = self.record_reported(branch, entry).await?;
            wrote = seq.or(wrote);
        }
        Ok(wrote)
    }

    /// Records one reported entry; the seq of a link's step that wrote.
    async fn record_reported(
        &self,
        branch: &BranchCtx,
        entry: Reported,
    ) -> Result<Option<i64>, String> {
        match entry {
            Reported::Event(e) => {
                let payload = world_event_payload(e);
                self.record(branch, None, EventKind::WorldEvent, &payload)
                    .await
                    .map(|_| None)
            }
            Reported::Link(s) => {
                let wrote = s.facts.wrote_world();
                let payload = tool_call_payload(&s.tool, s.args, &s.outcome, None, s.facts);
                let seq =
                    (self.record_seq(branch, Some(LINK_ACTOR), EventKind::ToolCall, &payload))
                        .await?;
                Ok(wrote.then_some(seq))
            }
        }
    }
}
