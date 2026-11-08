//! The plant's areas: the press lines with their presses, the robot cells and the AMRs.
use arena_part::proto::{TwinArea, TwinItem};
use serde_json::Value;

use super::text;
use crate::checks::World;

pub fn lines(w: &World) -> TwinArea {
    let items = w
        .docs("plant", "line")
        .into_iter()
        .map(|l| line(w, l))
        .collect();
    TwinArea::new("lines", "Press lines", "plant", items)
}

fn line(w: &World, l: &Value) -> TwinItem {
    let id = text(&l["id"]);
    let press = l["machines"][0].as_str().unwrap_or_default();
    let machine = w.doc("plant", &format!("machine/{press}"));
    let status = text(&l["status"]);
    let tripped = matches!(
        text(&machine["status"]),
        "tripped" | "locked-out" | "maintenance"
    );
    let mut it = TwinItem::new(
        format!("line:{id}"),
        text(&l["name"]),
        status,
        status != "running" || tripped,
    );
    let speed = l["boost"].as_f64().unwrap_or(1.0) * 100.0;
    it.detail = Some(format!(
        "{press} {}, {speed:.0}% speed",
        text(&machine["status"])
    ));
    it.qty = l["rate_per_hour"].as_f64();
    it.unit = Some("parts/h".into());
    it
}

pub fn cells(w: &World) -> TwinArea {
    let items = w
        .docs("plant", "cell")
        .into_iter()
        .map(|c| cell(w, c))
        .collect();
    TwinArea::new("cells", "Robot cell", "robot-cell", items)
}

fn cell(w: &World, c: &Value) -> TwinItem {
    let id = text(&c["id"]);
    let status = text(&c["status"]);
    let staffed = w.doc("roster", &format!("station/{id}"))["staffed"] == true;
    let abnormal = status != "running" || !staffed;
    let mut it = TwinItem::new(format!("cell:{id}"), text(&c["name"]), status, abnormal);
    let operator = if staffed { "staffed" } else { "not staffed" };
    it.detail = Some(format!(
        "{} kW, {operator}, zone {}",
        c["kw"],
        text(&c["zone"])
    ));
    it.qty = c["parts"].as_f64();
    it.unit = Some("parts".into());
    it.link = c["zone"].as_str().map(|z| format!("zone:{z}"));
    it
}

pub fn amrs(w: &World) -> TwinArea {
    let items = (w.docs("plant", "robot").into_iter())
        .map(|r| {
            let mut it = TwinItem::new(
                format!("amr:{}", text(&r["id"])),
                text(&r["name"]),
                text(&r["status"]),
                false,
            );
            it.detail = r["carrying"].as_str().map(|c| format!("carrying {c}"));
            it
        })
        .collect();
    TwinArea::new("amrs", "AMRs", "amr", items)
}
