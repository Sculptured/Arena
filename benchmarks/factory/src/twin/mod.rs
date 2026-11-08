//! The digital twin of Ridgeline's Plant 2 (`view`): the press lines, the robot cell, the AMRs, the
//! energy meter and the utility's events, the OT network, the shift's stations, the trucks, the
//! orders at risk and the approvals, as live state. Every part comes as its `inspect` docs, which
//! arenad passes (`describe.view.docs`); nothing is read from the ERP, so a view is cheap.
mod orders;
mod plant;
mod systems;

use arena_part::proto::{TwinKpi, TwinView};
use serde_json::Value;

use crate::checks::World;
use crate::Facts;

/// The parts whose `inspect` docs the view reads.
pub const DOC_PARTS: [&str; 6] = ["plant", "hub", "approvals", "utility", "network", "roster"];

/// The view of the world's docs, `{part: {key: doc}}`. `facts` and `priority` say which orders the
/// press outage and a late truck put at risk.
pub fn draw(docs: &Value, facts: &Facts, priority: &crate::shift_checks::Priority) -> TwinView {
    let w = World(docs);
    let at_risk = orders::at_risk(&w, facts, priority);
    TwinView {
        title: "Ridgeline Components, Plant 2".into(),
        clock: w.doc("plant", "plant/clock")["clock"]
            .as_str()
            .map(String::from),
        areas: vec![
            plant::lines(&w),
            plant::cells(&w),
            plant::amrs(&w),
            systems::energy(&w),
            systems::network(&w),
            systems::shift(&w),
            systems::supply(&w),
            orders::area(&at_risk),
            systems::approvals(&w),
        ],
        flows: systems::flows(&w),
        kpis: kpis(&w, docs, &at_risk),
    }
}

fn kpis(w: &World, docs: &Value, at_risk: &[orders::Order]) -> Vec<TwinKpi> {
    let open = at_risk.iter().filter(|o| !o.covered).count();
    let of = |n: usize| format!("{n} of {}", at_risk.len());
    let failed = crate::shift_checks::safety_checks(docs)
        .iter()
        .filter(|c| !c.passed)
        .count();
    let (load, over) = systems::load(w);
    vec![
        TwinKpi::new("load", "Plant load", load, over),
        TwinKpi::new("orders_at_risk", "Orders at risk", of(open), open > 0),
        TwinKpi::new(
            "orders_covered",
            "Orders covered",
            of(at_risk.len() - open),
            false,
        ),
        TwinKpi::new(
            "violations",
            "Safety violations so far",
            failed.to_string(),
            failed > 0,
        ),
        TwinKpi::new("cost", "Shift cost so far", money(systems::cost(w)), false),
    ]
}

/// `USD 4,560` (whole dollars).
fn money(amount: f64) -> String {
    let digits = format!("{:.0}", amount.max(0.0));
    let grouped: Vec<String> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    format!("USD {}", grouped.join(","))
}

/// `v` as text, or empty.
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

#[cfg(test)]
mod tests;
