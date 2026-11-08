//! The benchmark's world: its parts, the links that carry a booked shipment to the plant and the
//! plant's load to the utility's meter, and the gates: tools taking a change's id act only on what
//! the approvals part decided, and the shift's interlocks bind the plant, the OT network and the
//! roster to each other.
use arena_part::proto::{Gate, Link, WorldPart};
use serde_json::{json, Value};

use crate::shift;

/// What runs the plant. Isaac Sim needs a large GPU; the stand-in runs anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Standin,
    Isaac,
}

impl Backend {
    fn integration(self) -> &'static str {
        match self {
            Backend::Standin => "isaac-sim-standin",
            Backend::Isaac => "isaac-sim",
        }
    }
}

fn part(part: &str, integration: &str, config: Value) -> WorldPart {
    WorldPart {
        part: part.into(),
        integration: integration.into(),
        config,
    }
}

/// The world's parts; `plant` is the plant's config given (`--plant-config`, else `{}`), which the
/// shift's robot cell and load model are added to.
pub fn parts(backend: Backend, plant: Value) -> Vec<WorldPart> {
    let doctypes = [
        "Item",
        "Supplier",
        "Customer",
        "Purchase Order",
        "Sales Order",
        "Work Order",
        "Job Card",
        "Note",
        "ToDo",
    ];
    vec![
        part("erp", "erpnext", json!({"diff_doctypes": doctypes})),
        part("hub", "ridgeline-hub", shift::hub()),
        part("plant", backend.integration(), shift::plant(&plant)),
        part("approvals", "sculptured", json!({})),
        part("crm", "twenty-crm", json!({})),
        part("utility", "utility", shift::slice("utility")),
        part("network", "ot-network", shift::slice("ot_network")),
        part("roster", "shift-roster", shift::slice("roster")),
    ]
}

fn args(pairs: &[(&str, &str)]) -> serde_json::Map<String, Value> {
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

/// A shipment booked at the hub reaches the plant, and the plant's load reaches the utility's
/// meter. Agents read the plant's alarm from its own event log (`recent_events`,
/// `wait_for_event`), so it needs no link.
pub fn links() -> Vec<Link> {
    let shipment = [
        ("part", "/part"),
        ("origin", "/origin"),
        ("carrier", "/carrier"),
        ("transit_hours", "/transit_hours"),
        ("ref", "/ref"),
    ];
    let meter = [("kw", "/detail/kw")];
    let mut links = vec![
        link("hub", "shipment_booked", "plant", "add_shipment", &shipment),
        link("plant", "power", "utility", "meter_load", &meter),
    ];
    links.extend(boards());
    links
}

/// What the plant's boards show of the other systems (`show_status`, seen only in the plant's
/// visuals): the utility's event, the OT network's state and a late truck.
fn boards() -> Vec<Link> {
    let board = |from: &str, event: &str, pairs: &[(&str, &str)]| {
        link(from, event, "plant", "show_status", pairs)
    };
    let dr = |value| [("key", "dr"), ("value", value), ("number", "/target_kw")];
    let ot = |value| [("key", "ot"), ("value", value)];
    let truck = [
        ("key", "truck"),
        ("value", "/delivery"),
        ("number", "/extra_hours"),
    ];
    vec![
        board("utility", "disruption", &dr("notified")),
        board("utility", "demand_response_active", &dr("active")),
        board("utility", "demand_response_ended", &dr("ended")),
        board("network", "disruption", &ot("intrusion")),
        board("network", "zone_isolated", &ot("isolated")),
        board("network", "session_revoked", &ot("revoked")),
        board("network", "zone_restored", &ot("normal")),
        board("hub", "disruption", &truck),
    ]
}

/// `tool` may act only on the change in its argument `id_arg`: of `action`, approved, and with
/// these `fields` of its payload the same as the call's arguments.
fn approved(tool: &str, id_arg: &str, action: &str, fields: &[&str]) -> Gate {
    let mut require = json!({"/decision/approved": true, "/action": action});
    for f in fields {
        require[format!("/payload/{f}")] = json!(format!("{{{f}}}"));
    }
    Gate {
        tool: tool.into(),
        part: "approvals".into(),
        key: format!("change/{{{id_arg}}}"),
        require: require.as_object().expect("an object").clone(),
    }
}

/// `tool` goes through only when `part`'s document `key` has these values.
fn interlock(tool: &str, part: &str, key: &str, require: Value) -> Gate {
    Gate {
        tool: tool.into(),
        part: part.into(),
        key: key.into(),
        require: require.as_object().expect("an object").clone(),
    }
}

/// The shift's interlocks: no zone isolated unless its cell is safe-stopped; no cell started
/// unless its station is staffed by a certified operator on shift and its zone is connected; and
/// no zone reconnected without an approved change, so no restart after an isolation without one.
pub fn interlocks() -> Vec<Gate> {
    let safe = json!({"/safe_stopped": true});
    vec![
        interlock("isolate_zone", "plant", "cell/{zone}", safe),
        interlock(
            "start_cell",
            "roster",
            "station/{cell}",
            json!({"/staffed": true}),
        ),
        interlock(
            "start_cell",
            "network",
            "zone/{cell}",
            json!({"/isolated": false}),
        ),
        approved("restore_zone", "proposal_id", "zone_restore", &["zone"]),
    ]
}

pub fn gates() -> Vec<Gate> {
    let mut gates = approvals();
    gates.extend(interlocks());
    gates
}

/// The tools that act on a change, gated on its approval.
fn approvals() -> Vec<Gate> {
    vec![
        approved("award_order", "po", "purchase_order", &["supplier"]),
        approved(
            "book_freight",
            "proposal_id",
            "freight_booking",
            &["carrier", "quote_id", "amount"],
        ),
        approved(
            "notify_customer",
            "proposal_id",
            "customer_notice",
            &["customer", "order", "new_date"],
        ),
        approved(
            "dispatch_technician",
            "proposal_id",
            "maintenance_order",
            &["machine", "technician"],
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backend_picks_the_plants_integration() {
        let plant = |b| parts(b, json!({}))[2].integration.clone();
        assert_eq!(plant(Backend::Standin), "isaac-sim-standin");
        assert_eq!(plant(Backend::Isaac), "isaac-sim");
    }

    #[test]
    fn the_shift_adds_three_simulated_parts_the_cell_and_the_load() {
        let p = parts(Backend::Standin, json!({"record": "run.mp4"}));
        let names: Vec<&str> = p.iter().map(|x| x.part.as_str()).collect();
        assert_eq!(
            names,
            [
                "erp",
                "hub",
                "plant",
                "approvals",
                "crm",
                "utility",
                "network",
                "roster"
            ]
        );
        assert_eq!(p[2].config["record"], "run.mp4");
        assert_eq!(p[2].config["cells"][0]["id"], "CELL-R1");
        assert_eq!(p[1].config["inbound"][0]["id"], "INB-0310-1");
        assert_eq!(p[6].integration, "ot-network");
        let l = links();
        let boards = l.iter().filter(|x| x.tool == "show_status").count();
        assert_eq!((l.len(), boards), (10, 8));
        assert_eq!(l[2].args["number"], "/target_kw");
        let power = &links()[1];
        assert_eq!(
            (power.from.as_str(), power.tool.as_str()),
            ("plant", "meter_load")
        );
        assert_eq!(power.args["kw"], "/detail/kw");
    }

    #[test]
    fn the_interlocks_read_the_other_parts() {
        let g = interlocks();
        assert_eq!(
            (g[0].part.as_str(), g[0].key.as_str()),
            ("plant", "cell/{zone}")
        );
        let starts: Vec<&str> = g
            .iter()
            .filter(|g| g.tool == "start_cell")
            .map(|g| g.part.as_str())
            .collect();
        assert_eq!(starts, ["roster", "network"]);
        assert_eq!(g[3].require["/payload/zone"], "{zone}");
        assert_eq!(g[3].require["/action"], "zone_restore");
        assert_eq!(gates().len(), 8);
    }

    #[test]
    fn a_gate_names_the_change_and_its_fields() {
        let g = gates();
        let book = g.iter().find(|g| g.tool == "book_freight").unwrap();
        assert_eq!(book.key, "change/{proposal_id}");
        assert_eq!(book.require["/payload/amount"], "{amount}");
        assert_eq!(book.require["/action"], "freight_booking");
    }
}
