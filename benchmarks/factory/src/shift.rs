//! The shift task's data (`data/shift.json`, compiled in): the robot cell and the load model the
//! plant gets, the simulated utility, OT network and roster, the hub's raw-material truck, and the
//! four fronts a scenario may open. Unlike the factory example's data, it is in this repository.
use std::sync::OnceLock;

use serde_json::{json, Value};

const SHIFT: &str = include_str!("../data/shift.json");

/// The data, parsed once.
pub fn data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| serde_json::from_str(SHIFT).expect("data/shift.json is JSON"))
}

/// The plant clock at arena time 0, shared by every part.
pub fn plant_start() -> &'static str {
    data()["plant_start"]
        .as_str()
        .expect("shift.json has plant_start")
}

/// `base` with the fields of `extra` added (`extra`'s win).
pub fn merged(mut base: Value, extra: &Value) -> Value {
    let add = extra.as_object().cloned().unwrap_or_default();
    base.as_object_mut().expect("an object").extend(add);
    base
}

/// The plant's config: the robot cell and the load model, then the caller's (`--plant-config`).
pub fn plant(given: &Value) -> Value {
    merged(data()["plant"].clone(), given)
}

/// A simulated part's config: its slice of the data, on the plant's clock.
pub fn slice(name: &str) -> Value {
    merged(data()[name].clone(), &json!({"plant_start": plant_start()}))
}

/// The hub's config: its raw-material trucks.
pub fn hub() -> Value {
    json!({"inbound": data()["inbound"]})
}

/// A front's fixed values (`fronts.<name>`).
pub fn front(name: &str) -> &'static Value {
    &data()["fronts"][name]
}

/// How many days ahead an order counts as a priority of the shift.
pub fn priority_days() -> i64 {
    front("priority_days")
        .as_i64()
        .expect("fronts.priority_days")
}

/// The line the raw-material truck feeds.
pub fn truck_line() -> &'static str {
    let id = front("truck_late")["delivery"]
        .as_str()
        .expect("a delivery id");
    let trucks = data()["inbound"].as_array().expect("inbound is a list");
    let truck = trucks
        .iter()
        .find(|t| t["id"] == id)
        .expect("the late truck");
    truck["line"].as_str().expect("the truck's line")
}

/// The ids of the robot cells.
pub fn cells() -> Vec<&'static str> {
    let cells = data()["plant"]["cells"].as_array().expect("plant.cells");
    cells.iter().filter_map(|c| c["id"].as_str()).collect()
}

/// The lines on the plant's meter (Plant 2's press lines), which are on the roster too.
pub fn metered_lines() -> Vec<&'static str> {
    let lines = data()["plant"]["power"]["lines"].as_object();
    lines
        .into_iter()
        .flatten()
        .map(|(k, _)| k.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_names_what_the_world_needs() {
        assert_eq!(plant_start(), "2026-03-10T06:00:00");
        assert_eq!(cells(), ["CELL-R1"]);
        assert_eq!(metered_lines(), ["LINE-A", "LINE-B", "LINE-C"]);
        assert_eq!(truck_line(), "LINE-B");
        assert_eq!(priority_days(), 1);
        let plant = plant(&json!({"record": "x.mp4"}));
        assert_eq!(plant["record"], "x.mp4");
        assert_eq!(plant["power"]["base_kw"], 430);
        assert_eq!(slice("utility")["plant_start"], plant_start());
        assert_eq!(hub()["inbound"][0]["carrier"], "Lakeshore Freight");
    }
}
