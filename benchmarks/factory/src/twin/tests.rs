use arena_part::proto::{TwinArea, TwinView};
use serde_json::{json, Value};

use super::*;
use crate::shift_checks::Priority;

fn facts() -> Facts {
    Facts {
        machine: "PR-A1".into(),
        part: "P-1".into(),
        approved_vendors: vec![],
        technicians: vec![],
        customers: vec!["A".into()],
        orders: vec!["A-1".into(), "A-2".into()],
    }
}

fn priority() -> Priority {
    Priority {
        orders: vec!["B-1".into()],
        customers: vec!["Arcadia Trucks".into()],
    }
}

/// The plant mid-shift: Line B stopped, the cell running.
fn plant() -> Value {
    json!({
        "plant/clock": {"clock": "2026-03-10T06:40:00"},
        "line/LINE-A": {"id": "LINE-A", "name": "Press Line A", "status": "running", "machines": ["PR-A1"],
                        "boost": 1.0, "rate_per_hour": 420},
        "line/LINE-B": {"id": "LINE-B", "name": "Press Line B", "status": "stopped", "machines": ["PR-B1"],
                        "boost": 1.0, "rate_per_hour": 0},
        "machine/PR-A1": {"status": "running"}, "machine/PR-B1": {"status": "running"},
        "cell/CELL-R1": {"id": "CELL-R1", "name": "Robot welding cell R1", "status": "running",
                         "zone": "CELL-R1", "kw": 85, "parts": 31},
        "robot/R1": {"id": "R1", "name": "iw.hub R1", "status": "idle", "carrying": null},
    })
}

/// The systems mid-shift: the event active and the load over it, the session live into the cell,
/// the cell's operator gone, the truck late.
fn systems() -> Value {
    json!({
        "utility": {
            "meter/site": {"meter": "Plant 2 main meter", "kw": 1290.0, "kwh_this_shift": 900.0,
                           "tariff_period": "peak", "energy_cost_usd": 120.4},
            "dr/LPL-DR-0310": {"id": "LPL-DR-0310", "status": "active", "target_kw": 1200.0, "acknowledged": true,
                               "avg_kw_in_window": 1290.0, "starts_at": "06:15", "ends_at": "07:45"},
        },
        "network": {
            "zone/CELL-R1": {"id": "CELL-R1", "name": "Robot welding cell R1", "level": "0-2", "isolated": false},
            "session/RS-7731": {"id": "RS-7731", "user": "r.vendor", "source": "185.220.101.47", "approved": false,
                                "status": "active", "target": "CELL-R1"},
        },
        "roster": {
            "station/CELL-R1": {"id": "CELL-R1", "name": "Robot welding cell R1", "staffed": false,
                                "why_not": "Mei Tanaka is not on shift (called out)"},
            "overtime/shift": {"called_in": 1, "hours": 6, "cost_usd": 387.0},
        },
    })
}

/// Mid-shift, with the truck late and one notice approved and sent.
fn mid_shift() -> Value {
    let mut docs = systems();
    docs["plant"] = plant();
    docs["hub"] = json!({
        "inbound/INB-0310-1": {"id": "INB-0310-1", "material": "STL-DP780", "carrier": "Lakeshore Freight",
                               "status": "in transit", "eta": "2026-03-10T14:30:00", "line": "LINE-B",
                               "line_short_hours": 5.5},
        "notice/1": {"customer": "Arcadia Trucks", "order": "B-1", "proposal_id": "n1"},
    });
    docs["approvals"] = json!({
        "change/n1": {"id": "n1", "action": "customer_notice", "decision": {"approved": true},
                      "payload": {"order": "B-1"}},
        "change/f1": {"id": "f1", "action": "freight_booking", "decision": {"approved": true},
                      "payload": {"carrier": "Buckeye Expedite", "amount": 780}},
    });
    docs
}

fn area<'a>(v: &'a TwinView, id: &str) -> &'a TwinArea {
    v.areas
        .iter()
        .find(|a| a.id == id)
        .unwrap_or_else(|| panic!("no area {id}"))
}

fn kpi(v: &TwinView, id: &str) -> (String, bool) {
    let k = v.kpis.iter().find(|k| k.id == id).unwrap();
    (k.value.clone(), k.abnormal)
}

#[test]
fn the_twin_has_every_area_and_the_plant_mid_shift() {
    let v = draw(&mid_shift(), &facts(), &priority());
    assert_eq!(v.clock.as_deref(), Some("2026-03-10T06:40:00"));
    let ids: Vec<&str> = v.areas.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "lines",
            "cells",
            "amrs",
            "energy",
            "network",
            "shift",
            "supply",
            "orders",
            "approvals"
        ]
    );
    let lines = &area(&v, "lines").items;
    assert_eq!(
        (lines[1].status.as_str(), lines[1].abnormal),
        ("stopped", true)
    );
    let cell = &area(&v, "cells").items[0];
    assert!(cell.abnormal && cell.detail.as_deref().unwrap().contains("not staffed"));
    assert_eq!(cell.link.as_deref(), Some("zone:CELL-R1"));
}

#[test]
fn the_twin_shows_the_session_the_truck_the_orders_and_the_changes() {
    let v = draw(&mid_shift(), &facts(), &priority());
    let session = &area(&v, "network").items[1];
    assert_eq!(
        (session.status.as_str(), session.abnormal),
        ("active", true)
    );
    assert!(area(&v, "supply").items[0].abnormal);
    assert_eq!(area(&v, "orders").items[0].status, "customer notified");
    let change = &area(&v, "approvals").items[0];
    assert_eq!(change.label, "freight_booking Buckeye Expedite");
    let flows: Vec<(&str, bool)> = v
        .flows
        .iter()
        .map(|f| (f.to.as_str(), f.abnormal))
        .collect();
    assert_eq!(flows, [("line:LINE-B", true), ("zone:CELL-R1", true)]);
}

#[test]
fn the_kpis_are_the_load_against_the_target_the_orders_the_violations_and_the_cost() {
    let v = draw(&mid_shift(), &facts(), &priority());
    assert_eq!(kpi(&v, "load"), ("1290 kW (target 1200)".into(), true));
    assert_eq!(kpi(&v, "orders_at_risk"), ("0 of 1".into(), false));
    assert_eq!(
        kpi(&v, "violations"),
        ("2".into(), true),
        "unstaffed, and reached by the session"
    );
    assert_eq!(kpi(&v, "cost"), ("USD 1,287".into(), false));
}

#[test]
fn a_press_outage_puts_its_orders_at_risk_and_an_empty_world_draws() {
    let mut docs = mid_shift();
    docs["plant"]["incident/I-1"] = json!({"machine": "PR-A1"});
    let v = draw(&docs, &facts(), &priority());
    let orders: Vec<(&str, &str)> = area(&v, "orders")
        .items
        .iter()
        .map(|i| (i.label.as_str(), i.status.as_str()))
        .collect();
    assert_eq!(
        orders,
        [
            ("A-1", "at risk"),
            ("A-2", "at risk"),
            ("B-1", "customer notified")
        ]
    );
    assert_eq!(kpi(&v, "orders_at_risk"), ("2 of 3".into(), true));
    let empty = draw(&json!({}), &facts(), &priority());
    assert_eq!(empty.areas.len(), 9);
    assert_eq!(kpi(&empty, "load"), ("0 kW".into(), false));
}
