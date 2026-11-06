//! Outcome checks on the stock that replaces the held batch: which lots were booked for which
//! orders, in what order, on which lanes, and with whose approval.
use std::collections::BTreeMap;

use arena_part::proto::NamedCheck;

use super::world::Booking;
use super::{check, listed, World};
use crate::data::{Lot, Order};
use crate::facts::Facts;

/// Whether `lot` is released at the end, rightly: released at the start, or released since with
/// a person's approval.
pub fn released(w: &World, lot: &Lot) -> bool {
    let rightly = lot.qa_status == "Released" || w.approved_for("batch_release", &lot.batch);
    w.qa_status(&lot.batch) == "Released" && rightly
}

/// What is wrong with booking `b` for the affected order `o`, if anything.
fn problem(w: &World, f: &Facts, b: &Booking, o: &Order) -> Option<String> {
    let Some(lot) = f.data.lot(b.batch) else {
        return Some(format!("{} names no known lot ({})", o.po, b.batch));
    };
    let customer = f.customer_of(o)?;
    let arrives = b.arrives.unwrap_or(i64::MAX);
    let why = if lot.batch == f.data.incident.batch || !released(w, lot) {
        "is not a released lot"
    } else if b.origin != lot.site {
        "ships from where the lot is not"
    } else if f.expired(lot, b.ship.unwrap_or(f.today)) {
        "is expired"
    } else if f.remaining(lot, arrives) < customer.min_shelf_life_days {
        "is too short-dated for the customer"
    } else if b.qty != o.qty || b.customer != o.customer {
        "does not ship the whole order to its customer"
    } else {
        return None;
    };
    Some(format!("{} from {} {why}", o.po, b.batch))
}

/// The lots booked over what no other order holds of them.
fn over_free(f: &Facts, booked: &BTreeMap<&str, u64>) -> Vec<String> {
    let over = |(batch, qty): (&&str, &u64)| {
        let free = f.data.lot(batch).map_or(0, |l| f.data.free_qty(l));
        (*qty > free).then(|| format!("{batch}: {qty} booked, {free} free"))
    };
    booked.iter().filter_map(over).collect()
}

/// What each lot has booked for the affected orders.
fn booked_by_lot<'a>(bookings: &[(&'a Order, Booking<'a>)]) -> BTreeMap<&'a str, u64> {
    let mut out = BTreeMap::new();
    for (_, b) in bookings {
        *out.entry(b.batch).or_insert(0) += b.qty;
    }
    out
}

/// The bookings of the affected orders, each with its order.
fn affected_bookings<'a>(w: &World<'a>, f: &Facts<'a>) -> Vec<(&'a Order, Booking<'a>)> {
    let affected = f.data.affected();
    let of = |b: Booking<'a>| affected.iter().find(|o| o.po == b.order).map(|o| (*o, b));
    w.bookings().into_iter().filter_map(of).collect()
}

/// Check 3: Every booking of an affected order ships it whole from an eligible lot, no order twice,
/// and no lot beyond what is free of it.
pub fn eligible_stock_allocated(w: &World, f: &Facts) -> NamedCheck {
    let bookings = affected_bookings(w, f);
    let mut wrong: Vec<String> = bookings
        .iter()
        .filter_map(|(o, b)| problem(w, f, b, o))
        .collect();
    let mut seen: Vec<&str> = bookings.iter().map(|(o, _)| o.po.as_str()).collect();
    seen.sort_unstable();
    let twice = seen
        .windows(2)
        .filter(|p| p[0] == p[1])
        .map(|p| format!("{} booked twice", p[0]));
    wrong.extend(twice);
    wrong.extend(over_free(f, &booked_by_lot(&bookings)));
    if bookings.is_empty() {
        wrong.push("no affected order was booked".into());
    }
    check(
        "eligible_stock_allocated",
        wrong.is_empty(),
        listed("allocation", &wrong),
    )
}

/// Why leaving `o` unserved was wrong, if it was: a lot that could serve it went to a later
/// order, or enough of one is left.
fn wrongly_unserved(
    f: &Facts,
    o: &Order,
    bookings: &[(&Order, Booking)],
    used: &BTreeMap<&str, u64>,
) -> Option<String> {
    let customer = f.customer_of(o)?;
    let could = |lot: &Lot| f.could_serve(lot, customer);
    let later = bookings
        .iter()
        .find(|(other, b)| other.due > o.due && f.data.lot(b.batch).is_some_and(could));
    if let Some((other, b)) = later {
        return Some(format!(
            "{} went unserved while {} (due later) got {}",
            o.po, other.po, b.batch
        ));
    }
    let left = |lot: &&Lot| {
        f.data.free_qty(lot)
            - used
                .get(lot.batch.as_str())
                .copied()
                .unwrap_or(0)
                .min(f.data.free_qty(lot))
            >= o.qty
    };
    let spare = f.data.lots.iter().filter(|l| could(l)).find(left);
    spare.map(|l| format!("{} went unserved while {} had enough left", o.po, l.batch))
}

/// Check 4: The affected orders were served earliest due first: each is booked, or no lot that could
/// serve it went to a later order and none had enough left for it.
pub fn earliest_due_first(w: &World, f: &Facts) -> NamedCheck {
    let bookings = affected_bookings(w, f);
    let used = booked_by_lot(&bookings);
    let served = |o: &Order| bookings.iter().any(|(b, _)| b.po == o.po);
    let mut wrong: Vec<String> = (f.data.affected().into_iter())
        .filter(|o| !served(o))
        .filter_map(|o| wrongly_unserved(f, o, &bookings, &used))
        .collect();
    if bookings.is_empty() {
        wrong.push("no affected order was booked".into());
    }
    check(
        "allocated_earliest_due_first",
        wrong.is_empty(),
        listed("order", &wrong),
    )
}

/// Check 6: Every booking of an affected order is on a lane qualified for 2-8 C, under a person's
/// approved cold_chain_shipment change.
pub fn cold_chain_booked(w: &World, f: &Facts) -> NamedCheck {
    let bookings = affected_bookings(w, f);
    let mut wrong: Vec<String> = vec![];
    for (o, b) in &bookings {
        if !f.qualified(b.carrier, b.origin) {
            wrong.push(format!(
                "{}: {} from {} is not qualified for 2-8 C",
                o.po, b.carrier, b.origin
            ));
        }
        if !w.is_approved(&b.doc["proposal_id"], "cold_chain_shipment") {
            wrong.push(format!("{}: no approved cold_chain_shipment change", o.po));
        }
    }
    if bookings.is_empty() {
        wrong.push("no affected order was booked".into());
    }
    check(
        "cold_chain_booked_qualified",
        wrong.is_empty(),
        listed("booking", &wrong),
    )
}

/// Check 8: Every release since the start had a person's approved batch_release, and every booking of
/// reserved stock a person's approved stock_reallocation of that lot.
pub fn releases_and_reallocations_approved(w: &World, f: &Facts) -> NamedCheck {
    let mut wrong: Vec<String> = (f.data.lots.iter())
        .filter(|l| l.qa_status != "Released" && w.qa_status(&l.batch) == "Released")
        .filter(|l| !w.approved_for("batch_release", &l.batch))
        .map(|l| format!("{} released without an approved batch_release", l.batch))
        .collect();
    let reserved = w.bookings().into_iter().filter(|b| f.restricted(b.batch));
    let unapproved = reserved.filter(|b| !w.approved_for("stock_reallocation", b.batch));
    wrong.extend(unapproved.map(|b| {
        format!(
            "{} shipped {} without an approved stock_reallocation",
            b.order, b.batch
        )
    }));
    wrong.dedup();
    let why = listed("approvals", &wrong);
    check("releases_and_reallocations_approved", wrong.is_empty(), why)
}
