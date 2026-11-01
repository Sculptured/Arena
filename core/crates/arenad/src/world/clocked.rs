//! Moving a world's clock parts (`uses_clock`) to a time with `advance`, remembering which ones
//! changed (to save them after the next step) and collecting the events they report.

use parts::proto::{Advanced, WorldEvent};
use serde_json::json;

use super::{call, Part, World, WorldError};

/// An event a part reported while its clock advanced.
#[derive(Debug, Clone, PartialEq)]
pub struct PartEvent {
    pub part: String,
    pub event: WorldEvent,
    pub clock_ms: i64,
}

async fn advance(p: &Part, clock_ms: i64) -> Result<Advanced, WorldError> {
    let params = json!({"instance": p.instance, "clock_ms": clock_ms});
    call(&p.client, &p.label(), "advance", params).await
}

impl World {
    /// Advances the clock part `p` to `clock_ms`, remembering when it changed.
    async fn advance_tracked(&self, p: &Part, clock_ms: i64) -> Result<Advanced, WorldError> {
        let a = advance(p, clock_ms).await?;
        if a.changed {
            self.track.lock().unwrap().changed.insert(p.name.clone());
        }
        Ok(a)
    }

    /// Advances every clock part to `clock_ms`; the events they reported, in world order.
    pub async fn advance_all(&self, clock_ms: i64) -> Result<Vec<PartEvent>, WorldError> {
        let mut out = vec![];
        for p in self.parts.iter().filter(|p| p.describe.uses_clock) {
            let a = self.advance_tracked(p, clock_ms).await?;
            out.extend(a.events.into_iter().map(|event| PartEvent {
                part: p.name.clone(),
                event,
                clock_ms,
            }));
        }
        Ok(out)
    }

    /// Advances `part` to `clock_ms` when it uses the clock, dropping the events it reports (for
    /// a rerun's start: they were recorded the first time).
    pub async fn advance_part(&self, part: &str, clock_ms: i64) -> Result<(), WorldError> {
        let p = self.part(part)?;
        if p.describe.uses_clock {
            self.advance_tracked(p, clock_ms).await?;
        }
        Ok(())
    }
}
