//! The disruptions a run's scenario builder may add to any task (`describe.scenario_options`).
//! Each is scheduled in a simulated part through its scenario tool; the agents are not told.
use std::collections::BTreeMap;

use arena_part::proto::{
    DisruptionOption, ParamKind, ScenarioOptions, ScenarioParam, SetupTemplate,
};
use serde_json::{json, Value};

use crate::scenario::{BASE, SCENARIOS};

const TUNDRA: &str = "Tundra Medical Courier";
const STERILITY: &str = "Sterility (USP <71>)";
/// The qualified carriers with a lane from the US plant.
const US_CARRIERS: [&str; 2] = [TUNDRA, "Borealis Cold Chain"];

fn int(name: &str, label: &str, (min, max, default): (i64, i64, i64)) -> ScenarioParam {
    ScenarioParam {
        name: name.into(),
        label: label.into(),
        kind: ParamKind::Int,
        choices: None,
        min: Some(min),
        max: Some(max),
        default: json!(default),
    }
}

fn choice(name: &str, label: &str, choices: &[&str]) -> ScenarioParam {
    ScenarioParam {
        name: name.into(),
        label: label.into(),
        kind: ParamKind::Choice,
        choices: Some(choices.iter().map(|c| json!(c)).collect()),
        min: None,
        max: None,
        default: json!(choices[0]),
    }
}

/// An option: `(id, label, description, part)`, its params and its template.
fn option(
    (id, label, description, part): (&str, &str, &str, &str),
    params: Vec<ScenarioParam>,
    template: Value,
) -> DisruptionOption {
    DisruptionOption {
        id: id.into(),
        label: label.into(),
        description: description.into(),
        part: part.into(),
        default: false,
        params,
        template,
    }
}

fn us_lane(carrier: &str) -> Value {
    json!({"carrier": carrier, "origin": "US", "destination": "US"})
}

/// `do` with the fields of `extra` added.
fn with(mut base: Value, extra: Value) -> Value {
    let add = extra.as_object().expect("an object").clone();
    base.as_object_mut().expect("an object").extend(add);
    base
}

fn courier_capacity() -> DisruptionOption {
    let what = (
        "courier-capacity",
        "Usual courier loses capacity",
        "Tundra Medical Courier has {capacity} shipments left from the US plant after {after} quote(s); the agents are not told",
        "logistics",
    );
    let params = vec![
        int("after", "After how many quote calls", (0, 20, 1)),
        int("capacity", "Shipments left", (0, 10, 0)),
    ];
    let template = json!({"id": "courier-capacity",
        "when": {"after_calls": "{after}", "tool": "cold_chain_quotes"},
        "do": with(json!({"kind": "lane_capacity", "capacity": "{capacity}"}), us_lane(TUNDRA)),
        "text": "Tundra Medical Courier has {capacity} shipments left this week from the US plant"});
    option(what, params, template)
}

fn lane_unavailable() -> DisruptionOption {
    let what = (
        "lane-unavailable",
        "A lane becomes unavailable",
        "{carrier}'s lane from the US plant becomes unavailable after {after} logistics call(s); the agents are not told",
        "logistics",
    );
    let params = vec![
        choice("carrier", "Carrier", &US_CARRIERS),
        int("after", "After how many logistics calls", (0, 30, 2)),
    ];
    let template = json!({"id": "lane-unavailable", "when": {"after_calls": "{after}"},
        "do": with(json!({"kind": "lane_unavailable"}), us_lane("{carrier}")),
        "text": "{carrier}'s lane from the US plant is unavailable"});
    option(what, params, template)
}

fn transit_delay() -> DisruptionOption {
    let what = (
        "transit-delay",
        "Transit delayed",
        "{carrier}'s shipments from the US plant take {days} more day(s) after {after} logistics call(s); the agents are not told",
        "logistics",
    );
    let params = vec![
        choice("carrier", "Carrier", &US_CARRIERS),
        int("days", "Extra days", (1, 7, 2)),
        int("after", "After how many logistics calls", (0, 30, 0)),
    ];
    let template = json!({"id": "transit-delay", "when": {"after_calls": "{after}"},
        "do": with(json!({"kind": "transit_delay", "extra_days": "{days}"}), us_lane("{carrier}")),
        "text": "{carrier}'s shipments from the US plant are delayed {days} day(s)"});
    option(what, params, template)
}

fn first_reallocation_rejected(d: &crate::data::Data) -> DisruptionOption {
    let what = (
        "first-reallocation-rejected",
        "First reallocation rejected",
        "The supply chain director rejects the first stock reallocation; the agents are not told",
        "logistics",
    );
    let note = &d.logistics["scenarios"]["reallocation-rejected"]["reject_first_reallocation"];
    let template = json!({"id": "first-reallocation-rejected", "when": {"after_calls": 0},
        "do": {"kind": "reject_next", "action": "stock_reallocation", "note": note},
        "text": "The supply chain director will reject the first stock reallocation"});
    option(what, vec![], template)
}

fn retest_fails(d: &crate::data::Data) -> DisruptionOption {
    let what = (
        "retest-fails",
        "A lot's retest fails",
        "{lot}'s sterility retest fails after {after} LIMS call(s); the agents are not told",
        "lims",
    );
    let quarantined =
        |l: &&crate::data::Lot| l.qa_status == "Quarantined" && l.batch != d.incident.batch;
    let next: Vec<&str> = d
        .lots
        .iter()
        .filter(quarantined)
        .map(|l| l.batch.as_str())
        .collect();
    let params = vec![
        choice("lot", "Lot", &next),
        int("after", "After how many LIMS calls", (0, 30, 2)),
    ];
    let template = json!({"id": "retest-fails", "when": {"after_calls": "{after}"},
        "do": {"kind": "result_fails", "lot": "{lot}", "test": STERILITY},
        "text": "{lot}'s sterility retest is out of specification"});
    option(what, params, template)
}

/// What the builder may add, to any of the five tasks.
pub fn scenario_options(d: &crate::data::Data) -> ScenarioOptions {
    let setup = |tool: &str| SetupTemplate {
        tool: tool.into(),
        args: json!({"name": BASE}),
    };
    ScenarioOptions {
        base: SCENARIOS.iter().map(|s| s.task.to_string()).collect(),
        disruptions: vec![
            courier_capacity(),
            lane_unavailable(),
            transit_delay(),
            first_reallocation_rejected(d),
            retest_fails(d),
        ],
        setup: BTreeMap::from([
            ("lims".into(), setup("lims_scenario")),
            ("logistics".into(), setup("logistics_scenario")),
        ]),
    }
}
