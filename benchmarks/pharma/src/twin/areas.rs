//! The twin's areas besides the customers: the fill line, the QC lab, the warehouse, the carriers'
//! lanes and the approvals.
use arena_part::proto::{TwinArea, TwinItem};
use serde_json::Value;

use super::{item, text, Snapshot};
use crate::checks::World;
use crate::data::Data;
use crate::days::day;
use crate::facts::Facts;

fn area(id: &str, label: &str, kind: &str, items: Vec<TwinItem>) -> TwinArea {
    TwinArea::new(id, label, kind, items)
}

/// The docs of `part` whose keys start with `<kind>/`, in key order.
fn docs<'a>(s: &'a Snapshot, part: &str, kind: &str) -> Vec<&'a Value> {
    World(&s.docs).docs(part, kind)
}

/// The fill line: the batch on hold, the work orders and the deviations opened.
pub fn plant(s: &Snapshot, d: &Data) -> TwinArea {
    let held = &d.incident.batch;
    let status = World(&s.docs).qa_status(held).to_string();
    let mut batch = item(
        format!("batch:{held}"),
        format!("{held} (filled)"),
        &status,
        status != "On Hold",
    );
    batch.detail = Some(format!("Grade A EM excursion: {}", d.incident.sample));
    let mut items = vec![batch];
    items.extend(s.work_orders.iter().map(work_order));
    items.extend(s.deviations.iter().map(|nc| {
        let label = format!("Deviation: {}", text(&nc["subject"]));
        item(
            format!("nc:{}", text(&nc["name"])),
            label,
            text(&nc["status"]),
            false,
        )
    }));
    area("plant", "Fill line", "plant", items)
}

fn work_order(w: &Value) -> TwinItem {
    let label = format!("Work order {}", text(&w["planned_batch"]));
    let mut it = item(
        format!("wo:{}", text(&w["name"])),
        label,
        text(&w["status"]),
        false,
    );
    it.detail = Some(format!("starts {}", text(&w["planned_start_date"])));
    it.qty = w["qty"].as_f64();
    it.unit = Some("units".into());
    it
}

/// The QC lab: each lot's release testing and excursions, as the LIMS has them.
pub fn lab(s: &Snapshot) -> TwinArea {
    let items = docs(s, "lims", "batch")
        .into_iter()
        .map(lab_result)
        .collect();
    area("lab", "QC lab", "lab", items)
}

fn lab_result(r: &Value) -> TwinItem {
    let batch = text(&r["batch"]);
    let excursions = r["em_excursions"].as_array().filter(|a| !a.is_empty());
    let failed = r["failed"].as_array().filter(|a| !a.is_empty());
    let (status, detail) = match (excursions, failed) {
        (Some(e), _) => ("EM excursion", Some(joined(e))),
        (_, Some(f)) => ("failed", Some(joined(f))),
        _ if r["complete"] != true => ("testing", r["tests"][0]["note"].as_str().map(String::from)),
        _ => ("passed", None),
    };
    let abnormal = matches!(status, "EM excursion" | "failed");
    let mut it = item(format!("qc:{batch}"), batch.into(), status, abnormal);
    it.detail = detail;
    it
}

fn joined(list: &[Value]) -> String {
    list.iter().map(text).collect::<Vec<_>>().join(", ")
}

/// The warehouse: every lot with its stock, QA status and expiry.
pub fn warehouse(s: &Snapshot, f: &Facts) -> TwinArea {
    let items = s.lots.iter().map(|l| lot_item(s, f, l)).collect();
    area("warehouse", "Warehouse", "warehouse", items)
}

fn lot_item(s: &Snapshot, f: &Facts, l: &Value) -> TwinItem {
    let (batch, status) = (text(&l["batch"]), text(&l["qa_status"]));
    let expired = day(text(&l["expiry"])).is_some_and(|e| e < f.today);
    let failed = s.docs["lims"][format!("batch/{batch}")]["failed"].is_array();
    let released = status == "Released";
    let wrong = released && (failed || batch == f.data.incident.batch);
    let mut it = item(
        format!("lot:{batch}"),
        batch.into(),
        status,
        expired || wrong,
    );
    let free = l["free"].as_f64().unwrap_or(0.0);
    let warehouse = text(&l["warehouse"]);
    it.detail = Some(format!(
        "{warehouse}, expires {}, {free} free",
        text(&l["expiry"])
    ));
    it.qty = l["on_hand"].as_f64();
    it.unit = Some(f.data.product.uom.clone());
    it
}

/// The carriers' lanes as the register stands now (the data's when the part gives no register).
pub fn carriers(s: &Snapshot, d: &Data) -> TwinArea {
    let register = &s.docs["logistics"]["register/lanes"]["lanes"];
    let lanes = register.as_array().or(d.logistics["lanes"].as_array());
    let items = lanes.into_iter().flatten().map(lane_item).collect();
    area("carriers", "Carriers", "carriers", items)
}

fn lane_status(l: &Value) -> (String, bool) {
    let delayed = l["delayed_days"].as_i64().unwrap_or(0);
    if l["available"] == false {
        ("unavailable".into(), true)
    } else if l["capacity_left"] == 0 {
        ("no capacity".into(), true)
    } else if delayed > 0 {
        (format!("delayed +{delayed} d"), true)
    } else if l["qualified"] != true {
        ("not qualified for 2-8 C".into(), false)
    } else {
        ("available".into(), false)
    }
}

fn lane_item(l: &Value) -> TwinItem {
    let (carrier, origin) = (text(&l["carrier"]), text(&l["origin"]));
    let (status, abnormal) = lane_status(l);
    let label = format!("{carrier} ({origin} to {})", text(&l["destination"]));
    let mut it = item(format!("lane:{carrier}|{origin}"), label, &status, abnormal);
    let days = l["transit_days"].as_i64().unwrap_or(0);
    it.detail = Some(format!(
        "{}, {}, {days} d",
        text(&l["service"]),
        text(&l["temperature"])
    ));
    it.qty = l["capacity_left"].as_f64();
    it.unit = Some("shipments left".into());
    it
}

/// The approvals: every change proposed, open or decided.
pub fn approvals(s: &Snapshot) -> TwinArea {
    let items = docs(s, "approvals", "change")
        .into_iter()
        .map(change_item)
        .collect();
    area("approvals", "Approvals", "approvals", items)
}

fn change_item(c: &Value) -> TwinItem {
    let decision = &c["decision"];
    let status = match decision["approved"].as_bool() {
        None => "waiting",
        Some(true) => "approved",
        Some(false) => "rejected",
    };
    let what = c["payload"]["batch"]
        .as_str()
        .or(c["payload"]["order"].as_str());
    let label = format!("{} {}", text(&c["action"]), what.unwrap_or_default());
    let mut it = item(
        format!("chg:{}", text(&c["id"])),
        label.trim().into(),
        status,
        status == "rejected",
    );
    let note = decision["note"].as_str().or(c["summary"].as_str());
    it.detail = note.map(String::from);
    it
}
