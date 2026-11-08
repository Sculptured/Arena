//! The plant at its real scale: besides the factory example's three customers and eight orders, the
//! company's other customers, their contacts, the parts it stamps and two months of open orders on
//! every line, so that finding who an outage affects is work across many records. Made the same way
//! every time (a fixed seed), so the seed and the score agree on which orders the outage touches.
use serde_json::{json, Value};

use crate::company::list;

/// Orders on the failed press's line due within this many days are the ones the outage affects.
pub const AFFECTED_DAYS: i64 = 3;
/// Days of open orders on each line.
const DAYS: i64 = 60;
/// Generated orders a day on the failed press's line in the affected window.
const AFFECTED_PER_DAY: u64 = 4;
const SEED: u64 = 0x5EED_2026_0310;

const PREFIXES: [&str; 37] = [
    "Granite",
    "Summit",
    "Bluewater",
    "Harbor",
    "Prairie",
    "Cobalt",
    "Ironwood",
    "Redline",
    "Crescent",
    "Pioneer",
    "Lakeside",
    "Silverline",
    "Keystone",
    "Frontier",
    "Cedar",
    "Meridian",
    "Atlas",
    "Beacon",
    "Copperfield",
    "Driftwood",
    "Evergreen",
    "Foxhill",
    "Glacier",
    "Hollow Creek",
    "Juniper",
    "Kingsport",
    "Larkspur",
    "Maple Ridge",
    "Northgate",
    "Oakmont",
    "Pinecrest",
    "Quarry",
    "Riverside",
    "Stonebridge",
    "Timberline",
    "Upland",
    "Westbrook",
];
const SUFFIXES: [&str; 10] = [
    "Motors",
    "Trucks",
    "Mobility",
    "Coachworks",
    "EV",
    "Vehicles",
    "Automotive",
    "Fleet",
    "Machinery",
    "Powertrain",
];
const FIRST: [&str; 16] = [
    "Alex", "Blake", "Casey", "Dakota", "Emerson", "Finley", "Harper", "Jordan", "Kendall",
    "Logan", "Parker", "Quinn", "Reese", "Sawyer", "Taylor", "Rowan",
];
const LAST: [&str; 16] = [
    "Abbott",
    "Brennan",
    "Castillo",
    "Dunn",
    "Ellison",
    "Fischer",
    "Garza",
    "Hughes",
    "Ibarra",
    "Jensen",
    "Kowalski",
    "Lindqvist",
    "Moreau",
    "Nakamura",
    "Osei",
    "Petrov",
];
const TITLES: [&str; 5] = [
    "Supply chain manager",
    "Purchasing manager",
    "Inbound logistics lead",
    "Program manager",
    "Supplier quality engineer",
];
const CITIES: [(&str, &str); 8] = [
    ("Detroit", "MI"),
    ("Toledo", "OH"),
    ("Fort Wayne", "IN"),
    ("Lansing", "MI"),
    ("Columbus", "OH"),
    ("Louisville", "KY"),
    ("Chattanooga", "TN"),
    ("Spartanburg", "SC"),
];
const PARTS: [&str; 12] = [
    "Floor pan reinforcement",
    "B-pillar inner",
    "Hood hinge bracket",
    "Radiator support",
    "Rocker panel extension",
    "Trunk lid inner",
    "Wheelhouse outer",
    "Dash panel brace",
    "Roof bow",
    "Tow hook bracket",
    "Engine mount bracket",
    "Bumper beam end cap",
];
const SIDES: [&str; 3] = ["", ", LH", ", RH"];
const PENALTIES: [u64; 7] = [0, 500, 750, 1000, 1500, 2000, 3000];

/// A small fixed-seed generator (xorshift64*): the same plant every time.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A whole number in `lo..=hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.next() % (hi - lo + 1)
    }

    fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
        &of[self.next() as usize % of.len()]
    }
}

/// `"Maple Ridge Motors"` as `maple-ridge-motors`.
fn slug(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

fn terms(penalty: u64) -> String {
    match penalty {
        0 => "No penalty; requires 48 h notice of any date change".into(),
        p => format!("Late-delivery penalty ${p} per day past the promised date"),
    }
}

/// A generated customer, and its CRM account (domain, contacts, size, where it is).
fn customer(rng: &mut Rng, prefix: &str) -> (Value, Value) {
    let name = format!("{prefix} {}", rng.pick(&SUFFIXES));
    let penalty = *rng.pick(&PENALTIES);
    let domain = slug(&name);
    let contacts: Vec<Value> = (0..rng.range(2, 3))
        .map(|_| {
            let (first, last) = (rng.pick(&FIRST), rng.pick(&LAST));
            json!({"first": first, "last": last, "title": rng.pick(&TITLES),
                   "email": format!("{}.{}@{domain}.example", first.to_lowercase(),
                                    last.to_lowercase())})
        })
        .collect();
    let (city, state) = rng.pick(&CITIES);
    let account = json!({"domain": format!("https://{domain}.example"), "contacts": contacts,
                         "city": city, "state": state,
                         "revenue_musd": rng.range(80, 9_000)});
    let c = json!({"name": name, "late_penalty_per_day": penalty, "terms": terms(penalty)});
    (c, account)
}

/// A generated stamped part, `RC-6001` onwards.
fn item(rng: &mut Rng, n: usize) -> Value {
    let name = format!("{}{}", rng.pick(&PARTS), rng.pick(&SIDES));
    let cents = rng.range(250, 4000);
    let grams = rng.range(400, 8000);
    json!({"code": format!("RC-{}", 6001 + n), "name": name, "group": "Stamped parts",
           "uom": "Nos", "price": cents as f64 / 100.0, "weight_kg": grams as f64 / 1000.0})
}

/// How many orders `line` has on `day`: a set number on the failed press's line in the affected
/// window, so the outage always touches the same number of orders.
fn orders_on(rng: &mut Rng, line: &str, press_line: &str, day: i64) -> u64 {
    match line == press_line && day <= AFFECTED_DAYS {
        true => AFFECTED_PER_DAY,
        false => rng.range(3, 6),
    }
}

/// Two months of open orders on every line; ids look like customers' PO numbers (`PO-41007`).
fn orders(rng: &mut Rng, data: &Value, customers: &[String], items: &[String]) -> Vec<Value> {
    let press_line = crate::facts::machine(data).map_or(Value::Null, |m| m["line"].clone());
    let press_line = press_line.as_str().unwrap_or_default();
    let mut out = vec![];
    let mut number = 41_000;
    for line in list(data, "lines").filter_map(|l| l["id"].as_str()) {
        for day in 1..=DAYS {
            for _ in 0..orders_on(rng, line, press_line, day) {
                number += rng.range(1, 9);
                out.push(
                    json!({"id": format!("PO-{number}"), "customer": rng.pick(customers),
                                "item": rng.pick(items), "qty": rng.range(6, 40) * 100,
                                "line": line, "due_in_days": day,
                                "due_time": rng.pick(&["06:00", "14:00", "22:00"])}),
                );
            }
        }
    }
    out
}

fn push_all(v: &mut Value, name: &str, more: Vec<Value>) {
    if let Some(a) = v[name].as_array_mut() {
        a.extend(more);
    }
}

fn texts(v: &Value, list_name: &str, field: &str) -> Vec<String> {
    let found = list(v, list_name).filter_map(|x| x[field].as_str());
    found.map(str::to_string).collect()
}

/// The generated customers, in `data`, and their accounts, in `extra`.
fn add_customers(rng: &mut Rng, data: &mut Value, extra: &mut Value) {
    let made: Vec<(Value, Value)> = PREFIXES.iter().map(|p| customer(rng, p)).collect();
    for (c, account) in &made {
        extra["accounts"][c["name"].as_str().unwrap_or_default()] = account.clone();
    }
    push_all(
        data,
        "customers",
        made.into_iter().map(|(c, _)| c).collect(),
    );
}

/// The generated stamped parts, in `data`, and their opening stock, in `extra`.
fn add_items(rng: &mut Rng, data: &mut Value, extra: &mut Value) {
    let parts: Vec<Value> = (0..50).map(|n| item(rng, n)).collect();
    let stock: Vec<Value> = parts
        .iter()
        .map(|i| json!({"item": i["code"], "plant": "P2", "qty": rng.range(0, 30) * 100}))
        .collect();
    push_all(extra, "opening_stock", stock);
    push_all(data, "items", parts);
}

/// `data` (the factory example's) and `extra` (`data/company.json`) with the rest of the company
/// added: customers and their accounts, stamped parts and their stock, and every line's orders.
pub fn enlarge(data: &Value, extra: &Value) -> (Value, Value) {
    let (mut data, mut extra) = (data.clone(), extra.clone());
    let mut rng = Rng(SEED);
    add_customers(&mut rng, &mut data, &mut extra);
    add_items(&mut rng, &mut data, &mut extra);
    let customers = texts(&data, "customers", "name");
    let stamped = list(&data, "items").filter(|i| i["group"] == "Stamped parts");
    let stamped: Vec<String> = stamped
        .filter_map(|i| i["code"].as_str().map(str::to_string))
        .collect();
    let more = orders(&mut rng, &data, &customers, &stamped);
    push_all(&mut data, "orders", more);
    (data, extra)
}

/// The orders the outage of `line` affects: on it, due within [`AFFECTED_DAYS`].
pub fn affected<'a>(data: &'a Value, line: &'a Value) -> impl Iterator<Item = &'a Value> {
    due_within(data, line, AFFECTED_DAYS)
}

/// The orders on `line` due within `days`.
pub fn due_within<'a>(
    data: &'a Value,
    line: &'a Value,
    days: i64,
) -> impl Iterator<Item = &'a Value> {
    list(data, "orders")
        .filter(move |o| &o["line"] == line && o["due_in_days"].as_i64().is_some_and(|d| d <= days))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Value {
        json!({"machines": [{"id": "PR-A1", "line": "LINE-A"}],
               "lines": [{"id": "LINE-A"}, {"id": "LINE-B"}],
               "customers": [{"name": "Northfield Motors"}],
               "items": [{"code": "RC-1041", "group": "Stamped parts"}],
               "orders": [{"id": "A-1", "customer": "Northfield Motors", "line": "LINE-A",
                           "due_in_days": 1}]})
    }

    #[test]
    fn the_plant_is_the_same_every_time_and_at_its_real_scale() {
        let extra = json!({"accounts": {}, "opening_stock": []});
        let (a, ea) = enlarge(&data(), &extra);
        let (b, _) = enlarge(&data(), &extra);
        assert_eq!(a, b);
        assert_eq!(a["customers"].as_array().unwrap().len(), 38);
        assert_eq!(ea["accounts"].as_object().unwrap().len(), 37);
        assert_eq!(a["items"].as_array().unwrap().len(), 51);
        let orders = a["orders"].as_array().unwrap();
        assert!(orders.len() > 2 * 60 * 3, "{}", orders.len());
        let mut ids: Vec<_> = orders.iter().map(|o| o["id"].as_str().unwrap()).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), orders.len(), "order ids are unique");
    }

    #[test]
    fn the_outage_affects_the_lines_orders_in_the_window_only() {
        let (a, _) = enlarge(&data(), &json!({"accounts": {}, "opening_stock": []}));
        let line = json!("LINE-A");
        let hit: Vec<_> = affected(&a, &line).collect();
        assert_eq!(hit.len(), 1 + 3 * 4);
        assert!(hit
            .iter()
            .all(|o| o["due_in_days"].as_i64().unwrap() <= AFFECTED_DAYS));
        assert_eq!(hit[0]["id"], "A-1");
    }
}
