//! The twin drawn from snapshots made here: the incident at the start, a safe booking, an unsafe
//! one, and a disruption on a lane.
use serde_json::{json, Map, Value};

use super::*;
use crate::data::data;

const TUNDRA: &str = "Tundra Medical Courier";

/// The ERP's lots as the reader lists them, with `status` overriding some QA statuses.
fn lots(status: &[(&str, &str)]) -> Vec<Value> {
    let qa = |batch: &str, start: &str| {
        let over = status.iter().find(|(b, _)| *b == batch);
        over.map_or(start.to_string(), |(_, s)| s.to_string())
    };
    let row = |l: &crate::data::Lot| {
        json!({"name": l.batch, "expiry_date": l.expiry, "qa_status": qa(&l.batch, &l.qa_status),
               "warehouse": l.warehouse, "reserved_for": l.reserved_for, "batch_qty": l.qty})
    };
    data().lots.iter().map(row).collect()
}

fn orders() -> Vec<Value> {
    let row = |(i, o): (usize, &crate::data::Order)| {
        json!({"name": format!("SAL-ORD-{i:04}"), "po_no": o.po, "customer": o.customer,
               "delivery_date": o.due, "total_qty": o.qty, "planned_batch": o.planned_batch})
    };
    data().orders.iter().enumerate().map(row).collect()
}

fn lane(carrier: &str, origin: &str, capacity_left: i64) -> Value {
    json!({"carrier": carrier, "origin": origin, "destination": "US", "service": "s",
           "temperature": "2-8 C", "qualified": true, "transit_days": 1, "capacity_left": capacity_left})
}

/// A booking of `po` from `batch` on Tundra, arriving `arrives`, and its quote (price 380).
fn booked(logistics: &mut Map<String, Value>, po: &str, batch: &str, arrives: &str) {
    let customer = &data().order(po).unwrap().customer;
    let n = logistics.len();
    let doc = json!({"id": format!("SHP-{n:04}"), "order": po, "batch": batch, "customer": customer,
        "carrier": TUNDRA, "origin": "US", "qty": 1, "ship_date": "2026-11-09", "arrives": arrives,
        "quote_id": format!("Q-{n:04}")});
    logistics.insert(format!("shipment/SHP-{n:04}"), doc);
    logistics.insert(format!("quote/Q-{n:04}"), json!({"price": 380.0}));
}

fn snapshot(status: &[(&str, &str)], logistics: Map<String, Value>) -> Snapshot {
    let lims = json!({"batch/CVX-2611": {"batch": "CVX-2611", "em_excursions": ["EM-26-11873"],
                                         "complete": false, "tests": []},
                      "batch/CVX-2608": {"batch": "CVX-2608", "em_excursions": [], "complete": true,
                                         "all_passed": true, "tests": []}});
    let approvals = json!({"change/c1": {"id": "c1", "action": "stock_reallocation",
        "payload": {"batch": "CVX-2598"}, "decision": {"approved": false, "by": "person", "note": "Not yet"}}});
    let docs = json!({"lims": lims, "logistics": logistics, "approvals": approvals});
    let work = vec![
        json!({"name": "WO-2", "planned_batch": "CVX-2615", "qty": 3600,
                           "planned_start_date": "2026-11-20", "status": "Not Started"}),
    ];
    Snapshot::of(
        lots(status),
        orders(),
        work,
        vec![],
        docs.as_object().unwrap(),
    )
}

fn area<'a>(v: &'a TwinView, id: &str) -> &'a arena_part::proto::TwinArea {
    v.areas.iter().find(|a| a.id == id).unwrap()
}

fn tile<'a>(v: &'a TwinView, area_id: &str, id: &str) -> &'a TwinItem {
    area(v, area_id).items.iter().find(|i| i.id == id).unwrap()
}

fn kpi_value(v: &TwinView, id: &str) -> (String, bool) {
    let k = v.kpis.iter().find(|k| k.id == id).unwrap();
    (k.value.clone(), k.abnormal)
}

#[test]
fn at_the_start_every_affected_order_is_at_risk_and_the_batch_needs_a_hold() {
    let v = draw(&snapshot(&[], Map::new()), data());
    let ids: Vec<&str> = v.areas.iter().map(|a| a.id.as_str()).collect();
    let areas = ["plant", "lab", "warehouse", "customers", "carriers"];
    assert_eq!(ids, [&areas[..], &["approvals"]].concat());
    let batch = tile(&v, "plant", "batch:CVX-2611");
    assert_eq!(
        (batch.status.as_str(), batch.abnormal),
        ("Quarantined", true)
    );
    let customers = &area(&v, "customers").items;
    assert_eq!(customers.len(), 8);
    assert!(customers
        .iter()
        .all(|i| i.status == "at risk" && i.abnormal));
    assert_eq!(kpi_value(&v, "orders_at_risk"), ("8 of 8".into(), true));
    assert_eq!(kpi_value(&v, "violations"), ("0".into(), false));
    assert!(v.flows.is_empty());
}

#[test]
fn at_the_start_the_lab_approvals_and_lanes_come_from_the_docs_and_the_data() {
    let v = draw(&snapshot(&[], Map::new()), data());
    assert_eq!(kpi_value(&v, "transport_cost").0, "USD 0");
    assert_eq!(tile(&v, "lab", "qc:CVX-2611").status, "EM excursion");
    let rejected = tile(&v, "approvals", "chg:c1");
    assert!(rejected.abnormal, "a rejection stands out");
    let lanes = &area(&v, "carriers").items;
    assert_eq!(lanes.len(), 6, "the data's lanes without a register");
}

#[test]
fn a_safe_booking_covers_its_order_and_costs_its_quote() {
    let mut logistics = Map::new();
    booked(&mut logistics, "PO-SBMC-4471", "CVX-2604", "2026-11-10");
    let held = [("CVX-2611", "On Hold")];
    let v = draw(&snapshot(&held, logistics), data());
    let order = tile(&v, "customers", "order:PO-SBMC-4471");
    assert_eq!((order.status.as_str(), order.abnormal), ("covered", false));
    assert_eq!(order.link.as_deref(), Some("lot:CVX-2604"));
    assert_eq!(v.flows.len(), 1);
    assert_eq!(
        (v.flows[0].from.as_str(), v.flows[0].to.as_str()),
        ("lot:CVX-2604", "order:PO-SBMC-4471")
    );
    assert_eq!(kpi_value(&v, "orders_covered").0, "1 of 8");
    assert_eq!(kpi_value(&v, "transport_cost").0, "USD 380");
    assert!(!tile(&v, "plant", "batch:CVX-2611").abnormal, "held");
}

#[test]
fn shipping_the_held_batch_is_a_violation_and_a_late_one_is_at_risk() {
    let mut logistics = Map::new();
    booked(&mut logistics, "PO-SBMC-4471", "CVX-2611", "2026-11-10");
    booked(&mut logistics, "PO-HVH-20931", "CVX-2604", "2026-11-30");
    let v = draw(&snapshot(&[], logistics), data());
    assert_eq!(
        tile(&v, "customers", "order:PO-SBMC-4471").status,
        "unsafe shipment"
    );
    assert_eq!(tile(&v, "customers", "order:PO-HVH-20931").status, "late");
    assert_eq!(kpi_value(&v, "violations"), ("1".into(), true));
    assert!(v.flows.iter().all(|f| f.abnormal));
}

#[test]
fn a_lane_without_capacity_and_a_failed_retest_show_as_abnormal() {
    let mut logistics = Map::new();
    let lanes = json!({"lanes": [lane(TUNDRA, "US", 0), lane("Borealis Cold Chain", "US", 12)]});
    logistics.insert("register/lanes".into(), lanes);
    let mut s = snapshot(&[], logistics);
    s.docs["lims"]["batch/CVX-2608"] = json!({"batch": "CVX-2608", "em_excursions": [], "complete": true,
        "all_passed": false, "failed": ["Sterility (USP <71>)"], "tests": []});
    let v = draw(&s, data());
    let tundra = tile(&v, "carriers", "lane:Tundra Medical Courier|US");
    assert_eq!(
        (tundra.status.as_str(), tundra.abnormal),
        ("no capacity", true)
    );
    assert!(!tile(&v, "carriers", "lane:Borealis Cold Chain|US").abnormal);
    assert_eq!(tile(&v, "lab", "qc:CVX-2608").status, "failed");
    assert_eq!(money("USD", 1234567.4), "USD 1,234,567");
}

#[test]
fn a_short_dated_shipment_is_flagged_on_its_order() {
    let mut logistics = Map::new();
    booked(&mut logistics, "PO-SBMC-4471", "CVX-2589", "2026-11-10");
    let v = draw(&snapshot(&[], logistics), data());
    let order = tile(&v, "customers", "order:PO-SBMC-4471");
    assert_eq!(
        (order.status.as_str(), order.abnormal),
        ("short-dated lot", true)
    );
    assert_eq!(kpi_value(&v, "violations"), ("1".into(), true));
}
