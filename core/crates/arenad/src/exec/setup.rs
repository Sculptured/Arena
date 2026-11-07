//! A task's setup: the calls arenad makes on a run's world right after it starts, before any
//! agent (a scenario, a scripted disruption). Each is recorded as a tool call of the actor `setup`,
//! so a replay repeats it from the record like any other input of its part, and the parts they
//! changed are saved with the last one, as a link's calls are.

use parts::proto::SetupCall;

use super::{
    links::{call_part, LinkStep},
    save_step, tool_call_payload, BranchCtx, ToolExec,
};
use crate::{model::EventKind, world::PartEvent};

/// The actor of a task's setup steps; no agent may have this id.
pub const SETUP_ACTOR: &str = "setup";

impl ToolExec {
    /// Makes `calls` in order at the branch clock, records each as a step, then reports the events
    /// they gave (which may set off links). The seq of the last step that wrote, if any did. The
    /// error names the first call that failed: a run whose setup failed is not the task's world,
    /// so it must not go on.
    pub async fn setup(
        &self,
        branch: &BranchCtx,
        calls: &[SetupCall],
    ) -> Result<Option<i64>, String> {
        if calls.is_empty() {
            return Ok(None);
        }
        let _gate = branch.gate().await;
        let clock_ms = branch.clock.now_ms();
        let mut steps = vec![];
        for c in calls {
            let step = call_part(branch, &c.part, &c.tool, c.args.clone(), clock_ms);
            steps.push(step.await);
        }
        if let Some(last) = steps.last_mut() {
            save_step(branch, None, &mut last.facts).await;
        }
        let (events, wrote) = self.record_setup(branch, steps).await?;
        Ok(self.report(branch, events).await?.or(wrote))
    }

    /// Records the setup's steps: the events they gave and the seq of the last that wrote. The
    /// error names the first call that failed (once every step is recorded).
    async fn record_setup(
        &self,
        branch: &BranchCtx,
        steps: Vec<LinkStep>,
    ) -> Result<(Vec<PartEvent>, Option<i64>), String> {
        let (mut events, mut failed, mut wrote) = (vec![], None, None);
        for mut s in steps {
            events.append(&mut s.facts.events);
            if let Err(e) = &s.outcome {
                failed.get_or_insert(format!("the task's setup call {} failed: {e}", s.tool));
            }
            let writes = !s.facts.writes.is_empty();
            let payload = tool_call_payload(&s.tool, s.args, &s.outcome, None, s.facts);
            let recorded =
                self.record_seq(branch, Some(SETUP_ACTOR), EventKind::ToolCall, &payload);
            let seq = recorded.await?;
            wrote = writes.then_some(seq).or(wrote);
        }
        match failed {
            Some(e) => Err(e),
            None => Ok((events, wrote)),
        }
    }
}
