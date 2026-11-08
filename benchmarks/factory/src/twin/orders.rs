//! The orders at risk: those the press outage affects once PR-A1 is down, and the shift's priority
//! orders once their truck is late, each covered when its customer was sent a notice on an
//! approved change.
use arena_part::proto::{TwinArea, TwinItem};

use crate::checks::{unnoticed, World};
use crate::shift_checks::Priority;
use crate::Facts;

pub struct Order {
    pub id: String,
    pub why: &'static str,
    pub covered: bool,
}

fn press_down(w: &World, f: &Facts) -> bool {
    w.docs("plant", "incident")
        .iter()
        .any(|i| i["machine"] == f.machine.as_str())
}

fn truck_late(w: &World) -> bool {
    let trucks = w.docs("hub", "inbound");
    trucks
        .iter()
        .any(|t| t["line_short_hours"].as_f64().is_some_and(|h| h > 0.0))
}

pub fn at_risk(w: &World, f: &Facts, p: &Priority) -> Vec<Order> {
    let mut sets = vec![];
    if press_down(w, f) {
        sets.push(("press outage", &f.orders));
    }
    if truck_late(w) {
        sets.push(("late coil", &p.orders));
    }
    sets.into_iter()
        .flat_map(|(why, orders)| {
            let open = unnoticed(w, orders);
            orders.iter().map(move |o| Order {
                id: o.clone(),
                why,
                covered: !open.contains(&o.as_str()),
            })
        })
        .collect()
}

pub fn area(orders: &[Order]) -> TwinArea {
    let items = orders
        .iter()
        .map(|o| {
            let status = if o.covered {
                "customer notified"
            } else {
                "at risk"
            };
            let mut it =
                TwinItem::new(format!("order:{}", o.id), o.id.as_str(), status, !o.covered);
            it.detail = Some(o.why.into());
            it
        })
        .collect();
    TwinArea::new("orders", "Orders at risk", "customers", items)
}
