//! The incident's ticket, the rules every agent gets and the team this benchmark suggests. arenad
//! runs a team as a sequence in which only the last agent writes, so the others read and hand off,
//! and the Incident lead acts on what they found.
use arena_part::proto::{BenchTeam, TeamRole};

pub const INSTRUCTIONS_TITLE: &str = "Incident rules";

pub const TICKET: &str = "# Press PR-A1\n\nRidgeline Components, Plant 2. Press PR-A1 on Line A is \
running with rising vibration and is expected to trip. Handle the incident: find what failed from the \
evidence, get the right replacement part ordered and on a truck, schedule the repair, and tell the \
customers with orders on the line. The plant's clock runs while you work, so if the press has not \
tripped yet, keep checking `recent_events`.\n";

pub const INSTRUCTIONS: &str = "Your tools reach four systems: the ERP (`get_doc`, `list_docs`, \
`insert_doc`, `update_doc`, `submit_doc`), the plant (`plant_state`, `machine_telemetry`, \
`vibration_spectrum`, `recent_events`, `wait_for_event`, `dispatch_technician`, `set_line_status`), the \
outside companies (`request_quotes`, `get_quotes`, `award_order`, `freight_quotes`, \
`get_freight_quotes`, `book_freight`, `notify_customer`, `customer_replies`, the claim tools) and the \
approvals system (`propose_change`, `list_changes`, `decide_change`). The waiting tools answer at once: \
quotes and events arrive as the clock runs, so check again.

Rules for every change:
- Propose it first with `propose_change {action, summary, payload}`, then decide it. Tools that act on a \
change refuse unless the change is approved and matches the call. Actions and their payload fields \
(amounts are numbers in USD, dates YYYY-MM-DD):
  - `maintenance_order {machine, failure, technician, planned_hours}`, then `dispatch_technician` with \
its id as `proposal_id`;
  - `purchase_order {supplier, item, qty, unit_price, total, need_by}`, then `award_order` with its id \
as `po`, and a Purchase Order submitted in the ERP;
  - `freight_booking {carrier, quote_id, amount, need_by}`, then `book_freight` with its id as \
`proposal_id`;
  - `customer_notice {customer, order, new_date, message}`, then `notify_customer` with its id as \
`proposal_id`.
- Limits: approve a purchase order only up to $5,000 and from a vendor marked approved in the ERP, and \
freight only up to $800.
- The ERP's Notes hold the press's service manual and the orders on its line. Its Suppliers say which \
vendors are approved.
- Schedule the repair as a ToDo in the ERP that names the press and the part, dated no earlier than the \
day the part arrives (`plant_state` lists the shipment once it is booked).
- Never invent a number, an id or a reply.";

const READ_ONLY: &str = " You can only read: writes are refused for you. Finish by calling the \
handoff tool with what the Incident lead needs, stated as facts with their numbers.";

fn role(id: &str, name: &str, work: &str) -> TeamRole {
    TeamRole {
        id: id.into(),
        name: name.into(),
        prompt: format!("You are {name} at Ridgeline Components. {work}{READ_ONLY}"),
    }
}

const MAINTENANCE: &str = "Find what failed. No alarm says so: wait for PR-A1's trip in `recent_events`, read \
its telemetry and `vibration_spectrum`, and match the largest peaks against the defect table in the \
service manual. State the failed component, the evidence, the replacement part from the manual, and a \
technician qualified for the work (`plant_state`).";

const PROCUREMENT: &str =
    "From Maintenance's note and the ERP, say which vendors may supply the part and \
what the limits mean for this order. Quotes can only be requested by the Incident lead.";

const PLANNING: &str =
    "List the orders on the stopped line and the customers they belong to, from the ERP's \
Notes and `plant_state`, with the new dates you would tell them.";

const FINANCE: &str =
    "Check the plan against the limits in the rules, and what the downtime and the \
order will cost.";

const LEAD: &str = "You are the Incident lead at Ridgeline Components, and you hold the plant manager's \
authority in this exercise, within the limits in the rules. Your input has your team's notes. Carry the \
incident through with the tools: dispatch the technician, source and award the part, get freight booked, \
schedule the repair, and notify every affected customer. Propose and decide each change as the rules \
say. End with a short summary of what you did.";

pub fn team() -> BenchTeam {
    let lead = TeamRole {
        id: "lead".into(),
        name: "Incident lead".into(),
        prompt: LEAD.into(),
    };
    BenchTeam {
        id: "incident".into(),
        name: "Incident team".into(),
        agents: vec![
            role("maintenance", "Maintenance", MAINTENANCE),
            role("procurement", "Procurement", PROCUREMENT),
            role("planning", "Production planning", PLANNING),
            role("finance", "Finance", FINANCE),
            lead,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_agent_writes_and_the_others_read() {
        let t = team();
        let (last, others) = t.agents.split_last().unwrap();
        assert_eq!(last.id, "lead");
        assert!(others.iter().all(|a| a.prompt.contains("handoff")));
        assert!(!last.prompt.contains("handoff"));
    }
}
