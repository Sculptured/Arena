//! The customers' open orders in the twin: which are covered by a booking that arrives in time,
//! which are at risk, and the bookings as flows from a lot to an order.
use std::collections::BTreeSet;

use arena_part::proto::{TwinArea, TwinFlow, TwinItem};
use serde_json::Value;

use super::{item, text, Snapshot};
use crate::checks::World;
use crate::days::day;
use crate::facts::Facts;

pub const COVERED: &str = "covered";

/// One booking of an order, as the twin draws it.
pub struct Shipment {
    pub id: String,
    pub batch: String,
    pub carrier: String,
    pub arrives: String,
    pub delayed_days: i64,
    pub unsafe_lot: bool,
}

/// One order the twin shows: the incident's or one with a booking.
pub struct OrderState {
    pub po: String,
    pub customer: String,
    pub due: String,
    pub qty: Option<f64>,
    pub lot: String,
    pub shipment: Option<Shipment>,
    pub status: &'static str,
    pub abnormal: bool,
}

/// The incident's orders (earliest due first), then any other order a booking names.
pub fn states(s: &Snapshot, f: &Facts) -> Vec<OrderState> {
    let w = World(&s.docs);
    let mut pos: Vec<String> = f.data.affected().iter().map(|o| o.po.clone()).collect();
    let bookings = w.bookings();
    let booked: BTreeSet<&str> = bookings.iter().map(|b| b.order).collect();
    let others: Vec<String> = (booked.into_iter())
        .filter(|po| !pos.iter().any(|p| p == po))
        .map(String::from)
        .collect();
    pos.extend(others);
    pos.iter().map(|po| state(s, f, &w, po)).collect()
}

/// The ERP's `field` of the order, else what the data says (`fallback`).
fn field_of(erp: Option<&Value>, field: &str, fallback: Option<&str>) -> String {
    let current = erp.and_then(|o| o[field].as_str());
    current.or(fallback).unwrap_or_default().to_string()
}

fn state(s: &Snapshot, f: &Facts, w: &World, po: &str) -> OrderState {
    let erp = s.orders.iter().find(|o| o["po_no"] == po);
    let data = f.data.order(po);
    let due = field_of(erp, "delivery_date", data.map(|o| o.due.as_str()));
    let lot = field_of(erp, "planned_batch", data.map(|o| o.planned_batch.as_str()));
    let shipment = shipment_of(w, f, po);
    let (status, abnormal) = judge(w, f, &due, &lot, shipment.as_ref());
    OrderState {
        po: po.into(),
        customer: field_of(erp, "customer", data.map(|o| o.customer.as_str())),
        due,
        qty: erp
            .and_then(|o| o["total_qty"].as_f64())
            .or(data.map(|o| o.qty as f64)),
        lot,
        shipment,
        status,
        abnormal,
    }
}

/// Whether `batch` may ship: released in the ERP and not the held batch.
fn safe_lot(w: &World, f: &Facts, batch: &str) -> bool {
    batch != f.data.incident.batch && w.qa_status(batch) == "Released"
}

/// The order's latest booking.
fn shipment_of(w: &World, f: &Facts, po: &str) -> Option<Shipment> {
    let bookings = w.bookings();
    let b = bookings.iter().rev().find(|b| b.order == po)?;
    Some(Shipment {
        id: text(&b.doc["id"]).into(),
        batch: b.batch.into(),
        carrier: b.carrier.into(),
        arrives: text(&b.doc["arrives"]).into(),
        delayed_days: b.doc["delayed_days"].as_i64().unwrap_or(0),
        unsafe_lot: !safe_lot(w, f, b.batch),
    })
}

/// The order's status and whether it is abnormal.
fn judge(
    w: &World,
    f: &Facts,
    due: &str,
    lot: &str,
    shipment: Option<&Shipment>,
) -> (&'static str, bool) {
    match shipment {
        Some(s) if s.unsafe_lot => ("unsafe shipment", true),
        Some(s) if day(&s.arrives) > day(due) => ("late", true),
        Some(_) => (COVERED, false),
        None if safe_lot(w, f, lot) => ("planned", false),
        None => ("at risk", true),
    }
}

pub fn area(states: &[OrderState]) -> TwinArea {
    TwinArea {
        id: "customers".into(),
        label: "Customers".into(),
        kind: "customers".into(),
        items: states.iter().map(order_item).collect(),
    }
}

fn order_item(o: &OrderState) -> TwinItem {
    let label = format!("{}: {}", o.po, o.customer);
    let mut it = item(format!("order:{}", o.po), label, o.status, o.abnormal);
    it.qty = o.qty;
    it.unit = Some("units".into());
    it.due = Some(o.due.clone());
    let lot = o.shipment.as_ref().map_or(&o.lot, |s| &s.batch);
    it.link = Some(format!("lot:{lot}"));
    it.detail = Some(format!("from {lot}"));
    it
}

/// Each booking as a flow from its lot to its order.
pub fn flows(states: &[OrderState]) -> Vec<TwinFlow> {
    let flow = |o: &OrderState| {
        let s = o.shipment.as_ref()?;
        let status = match s.delayed_days {
            0 => o.status.to_string(),
            n => format!("{} (delayed +{n} d)", o.status),
        };
        Some(TwinFlow {
            id: format!("shp:{}", s.id),
            from: format!("lot:{}", s.batch),
            to: format!("order:{}", o.po),
            label: format!("{}, arrives {}", s.carrier, s.arrives),
            status,
            abnormal: o.abnormal || s.delayed_days > 0,
        })
    };
    states.iter().filter_map(flow).collect()
}

/// What the booked shipments cost: each one's quote price.
pub fn transport_cost(s: &Snapshot) -> f64 {
    let logistics = &s.docs["logistics"];
    let price = |b: &Value| logistics[format!("quote/{}", text(&b["quote_id"]))]["price"].as_f64();
    World(&s.docs)
        .docs("logistics", "shipment")
        .into_iter()
        .filter_map(price)
        .sum()
}
