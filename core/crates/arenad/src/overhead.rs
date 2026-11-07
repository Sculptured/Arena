//! Arena's own time, kept apart from the agents' and the world's so a run's numbers can estimate
//! production. Each piece of arena's work adds its milliseconds to the branch's ledger; the next
//! event the store records for the branch carries what built up since the one before, as
//! `overhead_ms`, and the ledger starts again. Summing `overhead_ms` over a branch's events gives
//! the time arena added to it.

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::Instant,
};

use serde::{Deserialize, Serialize};

/// The kinds of arena work that are timed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cat {
    /// Saving parts after a step (the world clock is paused meanwhile).
    Save,
    /// Starting a branch's world from a save, or by replay.
    Restore,
    /// Writing the record (the world clock is paused meanwhile).
    Record,
    /// The gateway's own handling of a model call, besides the provider's answer.
    Gateway,
    /// Gate checks: reading the document a gate asks for.
    Gate,
    /// Calls the world's links make after a step.
    Link,
    /// The MCP endpoint's own handling of a tool call, besides running it.
    Mcp,
    /// Waiting for the branch's lock, which keeps steps and saves in one order.
    Wait,
    /// Saving the final world and scoring it after the agents are done.
    Score,
    /// Someone paused the branch: nothing ran meanwhile.
    Pause,
}

/// Milliseconds per kind of arena work; a kind with none is left out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Overhead {
    #[serde(default, skip_serializing_if = "is_zero")]
    pub save: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub restore: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub record: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gateway: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gate: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub link: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub mcp: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wait: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub score: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pause: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Overhead {
    fn slot(&mut self, cat: Cat) -> &mut f64 {
        match cat {
            Cat::Save => &mut self.save,
            Cat::Restore => &mut self.restore,
            Cat::Record => &mut self.record,
            Cat::Gateway => &mut self.gateway,
            Cat::Gate => &mut self.gate,
            Cat::Link => &mut self.link,
            Cat::Mcp => &mut self.mcp,
            Cat::Wait => &mut self.wait,
            Cat::Score => &mut self.score,
            Cat::Pause => &mut self.pause,
        }
    }

    pub fn add(&mut self, cat: Cat, ms: f64) {
        *self.slot(cat) += ms.max(0.0);
    }

    /// Every kind of `other` added to this one.
    pub fn merge(&mut self, other: &Overhead) {
        for (cat, ms) in other.parts() {
            self.add(cat, ms);
        }
    }

    /// Each kind and its milliseconds.
    pub fn parts(&self) -> [(Cat, f64); 10] {
        [
            (Cat::Save, self.save),
            (Cat::Restore, self.restore),
            (Cat::Record, self.record),
            (Cat::Gateway, self.gateway),
            (Cat::Gate, self.gate),
            (Cat::Link, self.link),
            (Cat::Mcp, self.mcp),
            (Cat::Wait, self.wait),
            (Cat::Score, self.score),
            (Cat::Pause, self.pause),
        ]
    }

    pub fn total(&self) -> f64 {
        self.parts().iter().map(|(_, ms)| ms).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0.0
    }
}

/// Milliseconds since `since`, with the fraction kept.
pub fn ms_since(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.0
}

/// The overhead built up per branch and not yet recorded.
#[derive(Default)]
pub struct Ledger {
    pending: Mutex<HashMap<String, Overhead>>,
}

impl Ledger {
    pub fn add(&self, branch: &str, cat: Cat, ms: f64) {
        let mut p = self.pending.lock().unwrap();
        p.entry(branch.to_string()).or_default().add(cat, ms);
    }

    /// What built up for `branch` since the last take; None when nothing did.
    pub fn take(&self, branch: &str) -> Option<Overhead> {
        let taken = self.pending.lock().unwrap().remove(branch);
        taken.filter(|o| !o.is_empty())
    }
}

/// The one ledger of this process. Branch ids are unique, so branches of every store share it.
static LEDGER: LazyLock<Ledger> = LazyLock::new(Ledger::default);

/// Adds `ms` of arena work of kind `cat` to `branch`'s next recorded event.
pub fn add(branch: &str, cat: Cat, ms: f64) {
    LEDGER.add(branch, cat, ms);
}

/// Adds the time since `since` to `branch`, of kind `cat`.
pub fn add_since(branch: &str, cat: Cat, since: Instant) {
    add(branch, cat, ms_since(since));
}

/// What built up for `branch` since its last recorded event (None when nothing did).
pub fn take(branch: &str) -> Option<Overhead> {
    LEDGER.take(branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_hands_out_what_built_up_once() {
        let l = Ledger::default();
        assert_eq!(l.take("b"), None);
        l.add("b", Cat::Save, 2.0);
        l.add("b", Cat::Save, 1.5);
        l.add("b", Cat::Record, 0.25);
        l.add("other", Cat::Mcp, 9.0);
        let o = l.take("b").unwrap();
        assert_eq!((o.save, o.record, o.total()), (3.5, 0.25, 3.75));
        assert_eq!(l.take("b"), None);
        assert_eq!(l.take("other").unwrap().mcp, 9.0);
    }

    #[test]
    fn only_the_kinds_with_time_are_written() {
        let mut o = Overhead::default();
        o.add(Cat::Gate, 4.0);
        o.add(Cat::Wait, -1.0);
        assert_eq!(
            serde_json::to_value(o).unwrap(),
            serde_json::json!({"gate": 4.0})
        );
        let mut sum = Overhead::default();
        sum.merge(&o);
        sum.merge(&o);
        assert_eq!(sum.gate, 8.0);
    }
}
