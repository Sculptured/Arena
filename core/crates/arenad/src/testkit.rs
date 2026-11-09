//! Records unit tests build on: an event and a branch, with the fields a test doesn't look at
//! filled in.

use chrono::Utc;
use serde_json::{json, Value};

use crate::model::{Branch, BranchStatus, Event, EventKind, Team};

/// An event of branch `b`, recorded now.
pub fn event(seq: i64, agent: Option<&str>, kind: EventKind, payload: Value) -> Event {
    Event {
        branch_id: "b".into(),
        seq,
        at: Utc::now(),
        agent: agent.map(Into::into),
        kind,
        payload,
    }
}

/// Branch `b` of run `r`, team `T` with no agents, made now and not ended.
pub fn branch(status: BranchStatus) -> Branch {
    Branch {
        id: "b".into(),
        run_id: "r".into(),
        parent_id: None,
        start_seq: None,
        team: Team {
            id: "t".into(),
            name: "T".into(),
            agents: vec![],
        },
        changes: json!([]),
        fork_note: None,
        status,
        error: None,
        created_at: Utc::now(),
        ended_at: None,
        restore_mode: None,
        restored: vec![],
    }
}
