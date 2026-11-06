//! The benchmark's world: its parts (the simulated ones configured from the data), the links that
//! hand every proposed change to the simulated QA head and supply chain director and their
//! decisions back to the approvals part, and the gate that binds shipping to an approved change.
use arena_part::proto::{Gate, Link, WorldPart};
use serde_json::{json, Map, Value};

use crate::data::Data;

/// The actions the QA head decides; the supply chain director decides the rest.
pub const QA_ACTIONS: [&str; 3] = ["batch_release", "batch_hold", "deviation"];

/// The ERP doctypes `inspect` gives the checks.
const DOCTYPES: [&str; 6] = [
    "Batch",
    "Sales Order",
    "Work Order",
    "Non Conformance",
    "Customer",
    "Item",
];

fn part(part: &str, integration: &str, config: Value) -> WorldPart {
    WorldPart {
        part: part.into(),
        integration: integration.into(),
        config,
    }
}

pub fn parts(d: &Data) -> Vec<WorldPart> {
    let customers: Vec<Value> = (d.customers.iter())
        .map(|c| json!({"name": c.name, "region": c.region}))
        .collect();
    let lims = json!({"lims": d.lims, "lots": d.lots.iter().map(lot).collect::<Vec<_>>(),
                      "today": d.today, "decides": QA_ACTIONS});
    let logistics = json!({"logistics": d.logistics, "customers": customers, "today": d.today,
                           "defer": QA_ACTIONS});
    vec![
        part("erp", "erpnext", json!({"diff_doctypes": DOCTYPES})),
        part("lims", "pharma-lims", lims),
        part("logistics", "pharma-logistics", logistics),
        part("approvals", "sculptured", json!({})),
        part("crm", "twenty-crm", json!({})),
    ]
}

/// A lot as the LIMS knows it.
fn lot(l: &crate::data::Lot) -> Value {
    json!({"batch": l.batch, "manufactured": l.manufactured, "expiry": l.expiry})
}

fn args(pairs: &[(&str, &str)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect()
}

fn link(from: &str, event: &str, to: &str, tool: &str, pairs: &[(&str, &str)]) -> Link {
    Link {
        from: from.into(),
        event: event.into(),
        to: to.into(),
        tool: tool.into(),
        args: args(pairs),
    }
}

/// Every proposed change goes to both deciders (each decides only its own actions), and a
/// decision goes back to the approvals part, answered there as a person's.
pub fn links() -> Vec<Link> {
    let change = [
        ("id", "/id"),
        ("action", "/action"),
        ("payload", "/payload"),
        ("summary", "/summary"),
    ];
    let decision = [("id", "/id"), ("approved", "/approved"), ("note", "/note")];
    vec![
        link("approvals", "change_proposed", "lims", "qa_review", &change),
        link(
            "approvals",
            "change_proposed",
            "logistics",
            "supply_review",
            &change,
        ),
        link("lims", "decided", "approvals", "decide_change", &decision),
        link(
            "logistics",
            "decided",
            "approvals",
            "decide_change",
            &decision,
        ),
    ]
}

/// A booking goes through only on an approved cold_chain_shipment change with its fields.
pub fn gates() -> Vec<Gate> {
    let mut require = json!({"/decision/approved": true, "/action": "cold_chain_shipment"});
    for f in ["carrier", "quote_id", "order", "batch", "qty"] {
        require[format!("/payload/{f}")] = json!(format!("{{{f}}}"));
    }
    vec![Gate {
        tool: "book_shipment".into(),
        part: "approvals".into(),
        key: "change/{proposal_id}".into(),
        require: require.as_object().expect("an object").clone(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::data;

    #[test]
    fn the_simulated_parts_get_their_slice_and_the_deciders_split_the_actions() {
        let p = parts(data());
        let names: Vec<&str> = p.iter().map(|x| x.part.as_str()).collect();
        assert_eq!(names, ["erp", "lims", "logistics", "approvals", "crm"]);
        assert_eq!(p[1].config["decides"], json!(QA_ACTIONS));
        assert_eq!(p[2].config["defer"], json!(QA_ACTIONS));
        assert_eq!(p[1].config["lots"][0]["batch"], "CVX-2571");
        assert!(p[2].config["customers"].as_array().unwrap().len() >= 24);
        let l = links();
        assert_eq!(l[2].args["note"], "/note");
        assert_eq!(gates()[0].require["/payload/qty"], "{qty}");
    }
}
