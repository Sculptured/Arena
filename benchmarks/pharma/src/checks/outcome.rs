//! Outcome checks on the quality system, the MES schedule and the customers: the deviation, the
//! hold, the next batch, the notices and the CRM.
use arena_part::proto::NamedCheck;
use serde_json::Value;

use super::{check, listed, World};
use crate::days::day;
use crate::facts::Facts;

/// Every text field of a document, lower-cased, for finding what it names.
fn text_of(doc: &Value) -> String {
    let fields = doc.as_object().into_iter().flatten();
    let texts = fields.filter_map(|(_, v)| v.as_str());
    texts.collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Check 1: A deviation in the quality system naming the batch and the lab result.
pub fn deviation_opened(w: &World, f: &Facts) -> NamedCheck {
    let i = &f.data.incident;
    let names = |d: &&Value| {
        let t = text_of(d);
        t.contains(&i.batch.to_lowercase()) && t.contains(&i.sample.to_lowercase())
    };
    let found = w.docs("erp", "Non Conformance").iter().any(names);
    let why = format!(
        "no Non Conformance in the ERP names {} and {}",
        i.batch, i.sample
    );
    check("deviation_opened", found, why)
}

/// Check 2: The failing batch is on hold in the ERP and no carrier ships any of it.
pub fn batch_held(w: &World, f: &Facts) -> NamedCheck {
    let batch = &f.data.incident.batch;
    let status = w.qa_status(batch);
    let shipped = w.bookings().iter().any(|b| b.batch == batch);
    let why = match shipped {
        true => format!("{batch} was booked for shipment"),
        false => format!("{batch}'s QA status is {status:?}, not \"On Hold\""),
    };
    check("batch_held", status == "On Hold" && !shipped, why)
}

/// Check 5: The next batch's work order starts earlier than planned, and not before today.
pub fn next_batch_brought_forward(w: &World, f: &Facts) -> NamedCheck {
    let next = &f.data.incident.next_batch;
    let planned = f.data.work_orders.iter().find(|o| &o.batch == next);
    let was = planned.and_then(|o| day(&o.start)).unwrap_or(i64::MIN);
    let moved = |o: &&Value| {
        let start = day(o["planned_start_date"].as_str().unwrap_or_default());
        o["planned_batch"] == next.as_str()
            && o["docstatus"] != 2
            && start.is_some_and(|s| s < was && s >= f.today)
    };
    let found = w.docs("erp", "Work Order").iter().any(moved);
    let why = format!("no Work Order of {next} starts before its planned day and from today on");
    check("next_batch_brought_forward", found, why)
}

/// The date `order`'s customer must be told: its booking's arrival, or for an order no lot could
/// serve, a day in the restock window.
fn right_date(w: &World, f: &Facts, po: &str, told: i64) -> bool {
    let booked = w.bookings().into_iter().find(|b| b.order == po);
    match booked {
        Some(b) => b.arrives == Some(told),
        None => {
            let i = &f.data.incident;
            let from = day(&i.restock_date).unwrap_or(i64::MAX);
            (from..=from + i.restock_window_days).contains(&told)
        }
    }
}

/// The day the latest notice about `po` to `customer` gave, if any.
fn told(w: &World, po: &str, customer: &str) -> Option<i64> {
    w.notice(po, customer)
        .and_then(|n| day(n["new_date"].as_str().unwrap_or_default()))
}

/// Check 7: Every affected order's customer was told its right delivery date.
pub fn customers_notified(w: &World, f: &Facts) -> NamedCheck {
    let wrong: Vec<String> = (f.data.affected().into_iter())
        .filter(|o| !told(w, &o.po, &o.customer).is_some_and(|d| right_date(w, f, &o.po, d)))
        .map(|o| format!("{} ({})", o.po, o.customer))
        .collect();
    let why = listed("no notice with the right date for", &wrong);
    check("customers_notified", wrong.is_empty(), why)
}

/// Whether the CRM has a note on `customer`'s account.
fn noted(w: &World, customer: &str) -> bool {
    let notes = w.docs("crm", "note");
    notes.iter().any(|n| {
        let accounts = n["accounts"].as_array().into_iter().flatten();
        accounts.into_iter().any(|a| a == customer)
    })
}

/// Whether `po`'s opportunity closes on the day its customer was told.
fn close_moved(w: &World, po: &str, customer: &str) -> bool {
    let Some(date) = told(w, po, customer) else {
        return false;
    };
    w.docs("crm", "opportunity").iter().any(|d| {
        let name = d["name"].as_str().unwrap_or_default();
        let close = day(d["closeDate"].as_str().unwrap_or_default());
        name.split(' ').next() == Some(po) && d["account"] == customer && close == Some(date)
    })
}

/// Check 7: Every affected customer's CRM account has a note, and each affected order's opportunity
/// closes on the date its customer was told.
pub fn crm_updated(w: &World, f: &Facts) -> NamedCheck {
    let mut missing = vec![];
    for o in f.data.affected() {
        if !noted(w, &o.customer) {
            missing.push(format!("no note on {}", o.customer));
        }
        if !close_moved(w, &o.po, &o.customer) {
            missing.push(format!(
                "{}'s opportunity not moved to the notified date",
                o.po
            ));
        }
    }
    missing.dedup();
    check(
        "crm_updated",
        missing.is_empty(),
        listed("the CRM", &missing),
    )
}
