//! The benchmark's tasks: the press bearing failure (one front, scenario A of the demo) and the
//! shift (scenario B), whose fronts a run's scenario opens (`options`). The shift's setup calms
//! the plant, so no press wears; its world is otherwise the same.
use arena_part::proto::{SetupCall, TaskInfo};
use serde_json::json;

use crate::team;

/// The press bearing failure.
pub const TASK: &str = "press-a1-bearing";
/// The shift at Plant 2.
pub const SHIFT_TASK: &str = "shift";

pub fn tasks() -> Vec<TaskInfo> {
    let bearing = TaskInfo {
        id: TASK.into(),
        reason: "A press bearing fails: diagnose, source, approve, ship, schedule and notify \
                 every customer whose order the outage affects"
            .into(),
        ticket: team::TICKET.into(),
        setup: vec![],
    };
    let shift = TaskInfo {
        id: SHIFT_TASK.into(),
        reason: "One shift at Plant 2 with the fronts its scenario opens (energy, OT security, \
                 workforce, supply): keep the lines and the robot cell producing safely, meet the \
                 utility's target, contain the session, cover or notify the priority orders, and \
                 report the shift"
            .into(),
        ticket: team::SHIFT_TICKET.into(),
        setup: vec![SetupCall {
            part: "plant".into(),
            tool: "plant_scenario".into(),
            args: json!({"name": "calm"}),
        }],
    };
    vec![bearing, shift]
}
