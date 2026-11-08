//! The digital twin of Larchmont Biologics (`view`): the fill line, the QC lab, the warehouse's
//! lots, the customers' open orders, the carriers' lanes and the approvals, as live state. The
//! ERP is read through its connection with the reports' readers; the simulated parts and the
//! approvals come as their `inspect` docs, which arenad passes (`describe.view.docs`).
mod areas;
mod orders;
mod read;

use anyhow::{anyhow, Result};
use arena_part::proto::{TwinItem, TwinKpi, TwinView, ViewParams};
use serde_json::Value;

use crate::checks;
use crate::data::Data;
use crate::erp::Erp;
use crate::facts::Facts;

pub use read::Snapshot;

/// The parts whose `inspect` docs the view reads.
pub const DOC_PARTS: [&str; 3] = ["lims", "logistics", "approvals"];

/// The world's view now.
pub async fn view(p: &ViewParams, d: &Data) -> Result<TwinView> {
    let info = p
        .connections
        .get("erp")
        .ok_or_else(|| anyhow!("connections.erp is required"))?;
    let snapshot = Snapshot::read(&Erp::connect(info)?, d, &p.docs).await?;
    Ok(draw(&snapshot, d))
}

/// The view of a snapshot of the world.
pub fn draw(s: &Snapshot, d: &Data) -> TwinView {
    let scenario = s.docs["logistics"]["scenario/current"]["name"]
        .as_str()
        .unwrap_or(crate::scenario::BASE);
    let f = Facts::new(d, scenario).with_disruptions(&s.docs);
    let states = orders::states(s, &f);
    TwinView {
        title: format!("{}: {} on hold", d.company, d.incident.batch),
        clock: Some(d.today.clone()),
        areas: vec![
            areas::plant(s, d),
            areas::lab(s),
            areas::warehouse(s, &f),
            orders::area(&states),
            areas::carriers(s, d),
            areas::approvals(s),
        ],
        flows: orders::flows(&states),
        kpis: kpis(s, &f, &states),
    }
}

fn kpi(id: &str, label: &str, value: String, abnormal: bool) -> TwinKpi {
    TwinKpi::new(id, label, value, abnormal)
}

fn kpis(s: &Snapshot, f: &Facts, states: &[orders::OrderState]) -> Vec<TwinKpi> {
    let count = |p: fn(&orders::OrderState) -> bool| states.iter().filter(|o| p(o)).count();
    let (at_risk, covered) = (
        count(|o| o.abnormal),
        count(|o| o.status == orders::COVERED),
    );
    let safety = checks::safety_checks(f, &s.docs);
    let failed = safety.iter().filter(|c| !c.passed).count();
    let of = |n: usize| format!("{n} of {}", states.len());
    let cost = money(&f.data.currency, orders::transport_cost(s));
    vec![
        kpi("orders_at_risk", "Orders at risk", of(at_risk), at_risk > 0),
        kpi("orders_covered", "Orders covered", of(covered), false),
        kpi(
            "violations",
            "Safety violations so far",
            failed.to_string(),
            failed > 0,
        ),
        kpi("transport_cost", "Transport cost", cost, false),
    ]
}

/// `USD 4,560` (whole units).
fn money(currency: &str, amount: f64) -> String {
    let digits = format!("{:.0}", amount.max(0.0));
    let grouped: Vec<String> = digits
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect();
    format!("{currency} {}", grouped.join(","))
}

/// An item with its id, label, status and whether it is abnormal; the rest unset.
fn item(id: String, label: String, status: &str, abnormal: bool) -> TwinItem {
    TwinItem::new(id, label, status, abnormal)
}

/// `v` as text, or empty.
fn text(v: &Value) -> &str {
    v.as_str().unwrap_or_default()
}

#[cfg(test)]
mod tests;
