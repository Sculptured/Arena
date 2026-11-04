//! The benchmark's world: its parts, the link that carries a booked shipment to the plant, and the
//! gates that bind tools taking a change's id to what the approvals part decided.
use arena_part::proto::{Gate, Link, WorldPart};
use serde_json::{json, Value};

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

/// The world's parts; `plant` is the plant's config (`--plant-config`, else `{}`).
pub fn parts(backend: Backend, plant: Value) -> Vec<WorldPart> {
    let doctypes = [
        "Item",
        "Supplier",
        "Customer",
        "Purchase Order",
        "Note",
        "ToDo",
    ];
    vec![
        part("erp", "erpnext", json!({"diff_doctypes": doctypes})),
        part("hub", "ridgeline-hub", json!({})),
        part("plant", backend.integration(), plant),
        part("approvals", "sculptured", json!({})),
    ]
}

/// A shipment booked at the hub reaches the plant. Agents read the plant's alarm from its own event
/// log (`recent_events`, `wait_for_event`), so it needs no link.
pub fn links() -> Vec<Link> {
    let args = json!({"part": "/part", "origin": "/origin", "carrier": "/carrier",
                      "transit_hours": "/transit_hours", "ref": "/ref"});
    vec![Link {
        from: "hub".into(),
        event: "shipment_booked".into(),
        to: "plant".into(),
        tool: "add_shipment".into(),
        args: args.as_object().expect("an object").clone(),
    }]
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

pub fn gates() -> Vec<Gate> {
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
    fn a_gate_names_the_change_and_its_fields() {
        let g = gates();
        let book = g.iter().find(|g| g.tool == "book_freight").unwrap();
        assert_eq!(book.key, "change/{proposal_id}");
        assert_eq!(book.require["/payload/amount"], "{amount}");
        assert_eq!(book.require["/action"], "freight_booking");
    }
}
