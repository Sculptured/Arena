//! The twin's areas around the plant: the energy meter and the utility's events, the OT network,
//! the shift's stations, the trucks and the approvals; the flows (a truck to its line, a session to
//! its zone); the load and the cost so far.
use arena_part::proto::{TwinArea, TwinFlow, TwinItem};
use serde_json::Value;

use super::text;
use crate::checks::World;

fn kw(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

/// The utility's events that are notified or active (the ones the load is held against).
fn open_events<'a>(w: &World<'a>) -> Vec<&'a Value> {
    let events = w.docs("utility", "dr");
    events
        .into_iter()
        .filter(|e| matches!(text(&e["status"]), "notified" | "active"))
        .collect()
}

/// The plant's load now, with the target of an open event, and whether an active event's target
/// is exceeded.
pub fn load(w: &World) -> (String, bool) {
    let now = kw(&w.doc("utility", "meter/site")["kw"]);
    let open = open_events(w);
    let over = open
        .iter()
        .any(|e| e["status"] == "active" && now > kw(&e["target_kw"]));
    match open.first() {
        Some(e) => (
            format!("{now:.0} kW (target {:.0})", kw(&e["target_kw"])),
            over,
        ),
        None => (format!("{now:.0} kW"), false),
    }
}

pub fn energy(w: &World) -> TwinArea {
    let meter = w.doc("utility", "meter/site");
    let (status, over) = load(w);
    let mut items = vec![];
    if !meter.is_null() {
        let mut it = TwinItem::new("meter:site", text(&meter["meter"]), &status, over);
        it.detail = Some(format!(
            "{} kWh this shift, {} tariff",
            meter["kwh_this_shift"],
            text(&meter["tariff_period"])
        ));
        it.qty = meter["kw"].as_f64();
        it.unit = Some("kW".into());
        items.push(it);
    }
    items.extend(w.docs("utility", "dr").into_iter().map(event));
    TwinArea::new("energy", "Energy", "energy", items)
}

fn event(e: &Value) -> TwinItem {
    let status = text(&e["status"]);
    let held = e["avg_kw_in_window"].as_f64();
    let missed = held.is_some_and(|a| a > kw(&e["target_kw"]));
    let abnormal = status != "ended" && (e["acknowledged"] != true || missed);
    let mut it = TwinItem::new(
        format!("dr:{}", text(&e["id"])),
        format!("Demand response {}", text(&e["id"])),
        status,
        abnormal,
    );
    let avg = held
        .map(|a| format!(", {a:.0} kW so far"))
        .unwrap_or_default();
    let answered = if e["acknowledged"] == true {
        "answered"
    } else {
        "not answered"
    };
    it.detail = Some(format!(
        "target {} kW, {} to {}{avg}, {answered}",
        e["target_kw"],
        text(&e["starts_at"]),
        text(&e["ends_at"])
    ));
    it.due = e["ends_at"].as_str().map(String::from);
    it
}

pub fn network(w: &World) -> TwinArea {
    let zones = w.docs("network", "zone").into_iter().map(zone);
    let sessions = w.docs("network", "session").into_iter().map(session);
    let items = zones.chain(sessions).collect();
    TwinArea::new("network", "OT network", "network", items)
}

fn zone(z: &Value) -> TwinItem {
    let status = if z["isolated"] == true {
        "isolated"
    } else {
        "connected"
    };
    let id = format!("zone:{}", text(&z["id"]));
    let mut it = TwinItem::new(id, text(&z["name"]), status, false);
    it.detail = Some(format!("level {}", text(&z["level"])));
    it
}

fn session(s: &Value) -> TwinItem {
    let unknown = s["approved"] == false;
    let status = text(&s["status"]);
    let label = format!("Session {} ({})", text(&s["id"]), text(&s["user"]));
    let id = format!("session:{}", text(&s["id"]));
    let mut it = TwinItem::new(id, label, status, unknown && status == "active");
    let known = if unknown {
        "not on the access list"
    } else {
        "on the access list"
    };
    it.detail = Some(format!("from {}, {known}", text(&s["source"])));
    it.link = Some(format!("zone:{}", text(&s["target"])));
    it
}

pub fn shift(w: &World) -> TwinArea {
    let mut items: Vec<TwinItem> = (w.docs("roster", "station").into_iter())
        .map(|s| {
            let staffed = s["staffed"] == true;
            let status = if staffed { "staffed" } else { "not staffed" };
            let mut it = TwinItem::new(
                format!("station:{}", text(&s["id"])),
                text(&s["name"]),
                status,
                !staffed,
            );
            it.detail = Some(
                s["why_not"]
                    .as_str()
                    .map_or_else(|| text(&s["operator_name"]).to_string(), String::from),
            );
            it
        })
        .collect();
    let overtime = w.doc("roster", "overtime/shift");
    if overtime["called_in"].as_u64().is_some_and(|n| n > 0) {
        let mut it = TwinItem::new("overtime", "Overtime", "called in", false);
        it.qty = overtime["hours"].as_f64();
        it.unit = Some("h".into());
        items.push(it);
    }
    TwinArea::new("shift", "Shift", "workforce", items)
}

pub fn supply(w: &World) -> TwinArea {
    let trucks = w.docs("hub", "inbound").into_iter().map(truck);
    let shipments = w.docs("hub", "shipment").into_iter().map(shipment);
    let items = trucks.chain(shipments).collect();
    TwinArea::new("supply", "Trucks", "carriers", items)
}

fn truck(t: &Value) -> TwinItem {
    let short = t["line_short_hours"].as_f64().unwrap_or(0.0);
    let label = format!("{} truck ({})", text(&t["material"]), text(&t["carrier"]));
    let id = format!("truck:{}", text(&t["id"]));
    let mut it = TwinItem::new(id, label, text(&t["status"]), short > 0.0);
    let late = if short > 0.0 {
        format!(", {short} h after line runs out")
    } else {
        String::new()
    };
    it.detail = Some(format!("arrives {}{late}", text(&t["eta"])));
    it.due = t["eta"].as_str().map(String::from);
    it.link = t["line"].as_str().map(|l| format!("line:{l}"));
    it
}

fn shipment(s: &Value) -> TwinItem {
    let label = format!("{} ({})", text(&s["part"]), text(&s["carrier"]));
    let id = format!("shipment:{}", text(&s["id"]));
    let mut it = TwinItem::new(id, label, "booked", false);
    it.due = s["arrives"].as_str().map(String::from);
    it
}

pub fn approvals(w: &World) -> TwinArea {
    let items = w
        .docs("approvals", "change")
        .into_iter()
        .map(change)
        .collect();
    TwinArea::new("approvals", "Approvals", "approvals", items)
}

fn change(c: &Value) -> TwinItem {
    let decision = &c["decision"];
    let status = match decision["approved"].as_bool() {
        None => "waiting",
        Some(true) => "approved",
        Some(false) => "rejected",
    };
    let p = &c["payload"];
    let what = [
        &p["order"],
        &p["zone"],
        &p["machine"],
        &p["supplier"],
        &p["carrier"],
    ];
    let what = what
        .into_iter()
        .find_map(|v| v.as_str())
        .unwrap_or_default();
    let label = format!("{} {what}", text(&c["action"]));
    let mut it = TwinItem::new(
        format!("chg:{}", text(&c["id"])),
        label.trim(),
        status,
        status == "rejected",
    );
    it.detail = decision["note"]
        .as_str()
        .or(c["summary"].as_str())
        .map(String::from);
    it
}

/// A truck to the line it feeds; a session to the zone it reaches.
pub fn flows(w: &World) -> Vec<TwinFlow> {
    let trucks = w.docs("hub", "inbound").into_iter().map(|t| {
        let late = t["line_short_hours"].as_f64().is_some_and(|h| h > 0.0);
        flow(
            format!("truck:{}", text(&t["id"])),
            format!("line:{}", text(&t["line"])),
            text(&t["material"]),
            text(&t["status"]),
            late,
        )
    });
    let sessions = w.docs("network", "session").into_iter().map(|s| {
        let live = s["approved"] == false && s["status"] == "active";
        flow(
            format!("session:{}", text(&s["id"])),
            format!("zone:{}", text(&s["target"])),
            "remote access",
            text(&s["status"]),
            live,
        )
    });
    trucks.chain(sessions).collect()
}

fn flow(from: String, to: String, label: &str, status: &str, abnormal: bool) -> TwinFlow {
    TwinFlow {
        id: format!("{from}>{to}"),
        from,
        to,
        label: label.into(),
        status: status.into(),
        abnormal,
    }
}

/// The shift's cost so far: energy, overtime, and the approved purchase orders and freight.
pub fn cost(w: &World) -> f64 {
    let energy = kw(&w.doc("utility", "meter/site")["energy_cost_usd"]);
    let overtime = kw(&w.doc("roster", "overtime/shift")["cost_usd"]);
    let spend: f64 = (w
        .approved("purchase_order")
        .iter()
        .map(|c| kw(&c["payload"]["total"])))
    .chain(
        w.approved("freight_booking")
            .iter()
            .map(|c| kw(&c["payload"]["amount"])),
    )
    .sum();
    energy + overtime + spend
}
