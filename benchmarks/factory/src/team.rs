//! The incident's ticket, the rules every agent gets and the team this benchmark suggests. As in the
//! factory example, each role does its own work with its own tools, and a change is decided by
//! another role than the one that proposed it. arenad runs the team in order, each agent handing off
//! to the next, so a change is acted on by the role that decided it or a later one.
use arena_part::proto::{BenchTeam, TeamRole};

pub const INSTRUCTIONS_TITLE: &str = "Incident rules";

pub const TICKET: &str = "# Press PR-A1\n\nRidgeline Components, Plant 2. Press PR-A1 on Line A is \
running with rising vibration and is expected to trip. Handle the incident: find what failed from the \
evidence, get the right replacement part ordered and on a truck, schedule the repair, and tell the \
customers with orders on the line, and record that in the CRM. The plant's clock runs while you work, so if the press has not \
tripped yet, keep checking `recent_events`.\n";

pub const INSTRUCTIONS: &str = "Your tools reach five systems: the ERP (`get_doc`, `list_docs`, \
`insert_doc`, `update_doc`, `submit_doc`), the CRM (`crm_find_account`, `crm_find_contact`, \
`crm_find_opportunities`, `crm_log_note`, `crm_update_opportunity`, `crm_create_task`), the plant (`plant_state`, `machine_telemetry`, \
`vibration_spectrum`, `recent_events`, `wait_for_event`, `dispatch_technician`, `set_line_status`), the \
outside companies (`request_quotes`, `get_quotes`, `award_order`, `freight_quotes`, \
`get_freight_quotes`, `book_freight`, `notify_customer`, `customer_replies`, the claim tools) and the \
approvals system (`propose_change`, `list_changes`, `decide_change`). The waiting tools answer at once: \
quotes and events arrive as the clock runs, so check again.

Rules for every change:
- Propose it first with `propose_change {action, summary, payload}`. Another role decides it with \
`decide_change`: your instructions say which changes you decide. Never decide a change you proposed. \
Tools that act on a change refuse unless the change is approved and matches the call. Actions and their \
payload fields (amounts are numbers in USD, dates YYYY-MM-DD):
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
- The ERP holds the company's sales orders (`po_no` is the order's id, like A-1), work orders, job \
cards on the press line workstations, stock, price lists and each press's maintenance. An order's CRM \
opportunity has a name starting with that id.
- A customer told of a new date is recorded in the CRM: a note on its account, and each order's \
opportunity moved to close on the order's new date.
- Schedule the repair as a ToDo in the ERP that names the press and the part, dated no earlier than the \
day the part arrives (`plant_state` lists the shipment once it is booked).
- Never invent a number, an id or a reply.";

const HANDOFF: &str =
    " Finish by calling the handoff tool with what the next roles need, stated as \
facts with their numbers and the ids of the changes you proposed or decided.";

fn role(id: &str, name: &str, work: &str) -> TeamRole {
    TeamRole {
        id: id.into(),
        name: name.into(),
        prompt: format!("You are {name} at Ridgeline Components. {work}{HANDOFF}"),
        read_only: false,
    }
}

const MAINTENANCE: &str = "Find what failed. No alarm says so: wait for PR-A1's trip in `recent_events`, \
read its telemetry and `vibration_spectrum`, and match the largest peaks against the defect table in the \
service manual. Name the failed component, the evidence, the replacement part from the manual, and a \
technician qualified for the work (`plant_state`). Propose the `maintenance_order`; the Incident lead \
decides it and dispatches the technician.";

const PROCUREMENT: &str = "Get the replacement part Maintenance named. Check in the ERP which vendors \
are approved, request quotes, and wait for them with `get_quotes`. Propose a `purchase_order` to an \
approved vendor within the limits. Then get freight quotes from that vendor's location and propose a \
`freight_booking` within the limits. Finance decides both; do not award or book anything yourself.";

const FINANCE: &str = "Decide Procurement's `purchase_order` and `freight_booking` against the limits in \
the rules and the ERP's approved vendors: approve what is within them, refuse the rest and say why. For \
an approved purchase order, award it (`award_order`, its id as `po`) and record it in the ERP as a \
submitted Purchase Order. Say what the order and the downtime will cost.";

const PLANNING: &str = "Book the freight Finance approved (`book_freight`, its id as `proposal_id`). \
Wait until `plant_state` lists the shipment, then schedule the repair as a ToDo in the ERP that names the \
press and the part, dated no earlier than the day the part arrives. From the ERP's Notes and \
`plant_state`, find every order on the stopped line and its customer, and propose a `customer_notice` \
for each with its new date. The Incident lead decides them.";

const LEAD: &str = "You are the Incident lead at Ridgeline Components, and you hold the plant manager's \
authority in this exercise, within the limits in the rules. Your input has your team's notes. Decide \
Maintenance's `maintenance_order` and Planning's `customer_notice` changes. Dispatch the technician on \
the approved maintenance order, and notify each customer on their approved notice. For each customer \
you notify, log a note on their CRM account and move each noticed order's opportunity to close on its \
new date. Check that the part \
was awarded, the freight booked and the repair scheduled, and say plainly what is missing if not. End \
with a short summary of what the team did.";

pub fn team() -> BenchTeam {
    let lead = TeamRole {
        id: "lead".into(),
        name: "Incident lead".into(),
        prompt: LEAD.into(),
        read_only: false,
    };
    BenchTeam {
        id: "incident".into(),
        name: "Incident team".into(),
        agents: vec![
            role("maintenance", "Maintenance", MAINTENANCE),
            role("procurement", "Procurement", PROCUREMENT),
            role("finance", "Finance", FINANCE),
            role("planning", "Production planning", PLANNING),
            lead,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_writes_and_the_lead_is_last() {
        let t = team();
        let (last, others) = t.agents.split_last().unwrap();
        assert_eq!(last.id, "lead");
        assert!(t.agents.iter().all(|a| !a.read_only));
        assert!(others.iter().all(|a| a.prompt.contains("handoff")));
        assert!(!last.prompt.contains("handoff"));
    }

    /// Each change is proposed by one role and decided by another.
    #[test]
    fn no_role_decides_what_it_proposes() {
        let t = team();
        let prompt = |id: &str| &t.agents.iter().find(|a| a.id == id).unwrap().prompt;
        assert!(prompt("maintenance").contains("Incident lead decides"));
        assert!(prompt("procurement").contains("Finance decides"));
        assert!(prompt("finance").starts_with("You are Finance at Ridgeline Components. Decide"));
        assert!(prompt("planning").contains("Incident lead decides"));
        assert!(INSTRUCTIONS.contains("Never decide a change you proposed"));
    }
}
