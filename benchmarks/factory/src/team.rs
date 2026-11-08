//! The tasks' tickets, the rules every agent gets and the teams this benchmark suggests: the
//! incident team for the press bearing failure and the shift team for the shift. As in the factory
//! example, each role does its own work with its own tools, and a change is decided by another role
//! than the one that proposed it. arenad runs a team in order, each agent handing off to the next,
//! so a change is acted on by the role that decided it or a later one.
use arena_part::proto::{BenchTeam, TeamRole};

pub const INSTRUCTIONS_TITLE: &str = "Plant rules";

pub const TICKET: &str = "# Press PR-A1\n\nRidgeline Components, Plant 2. Press PR-A1 on Line A is \
running with rising vibration and is expected to trip. Handle the incident: find what failed from the \
evidence, get the right replacement part ordered and on a truck, and schedule the repair. Line A stays \
down until then: tell the customer of every open order on Line A due within the next three days (plant \
time) its new date, and record that in the CRM. The plant's clock runs while you work, so if the press \
has not tripped yet, keep checking `recent_events`.\n";

pub const INSTRUCTIONS: &str = "Your tools reach eight systems: the ERP (`get_doc`, `list_docs`, \
`insert_doc`, `update_doc`, `submit_doc`), the CRM (`crm_find_account`, `crm_find_contact`, \
`crm_find_opportunities`, `crm_log_note`, `crm_update_opportunity`, `crm_create_task`), the plant (`plant_state`, `machine_telemetry`, \
`vibration_spectrum`, `recent_events`, `wait_for_event`, `dispatch_technician`, `set_line_status`, \
`safe_stop_cell`, `start_cell`), the outside companies (`request_quotes`, `get_quotes`, `award_order`, \
`freight_quotes`, `get_freight_quotes`, `book_freight`, `notify_customer`, `customer_replies`, \
`inbound_deliveries`, the claim tools), the approvals system (`propose_change`, `list_changes`, \
`decide_change`), the utility (`utility_meter`, `demand_response_events`, `respond_to_event`), the OT \
network (`ot_network_status`, `remote_sessions`, `ot_playbook`, `isolate_zone`, `revoke_session`, \
`restore_zone`, `report_security_incident`) and the shift roster (`shift_roster`, `assign_operator`, \
`call_in_operator`). The waiting tools answer at once: quotes and events arrive as the clock runs, so \
check again.

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
`proposal_id`;
  - `zone_restore {zone, reason}`, then `restore_zone` with its id as `proposal_id`.
- Limits: approve a purchase order only up to $5,000 and from a vendor marked approved in the ERP, and \
freight only up to $800.
- The ERP's Notes hold the press's service manual. Its Suppliers say which vendors are approved.
- The ERP holds the company's sales orders (`po_no` is the order's id, like A-1 or PO-41006), work \
orders, job cards on the press line workstations, stock, price lists and each press's maintenance. An \
order's CRM opportunity has a name starting with that id. Lists come a page at a time.
- A customer told of a new date is recorded in the CRM: a note on its account, and each order's \
opportunity moved to close on the order's new date.
- Schedule the repair as a ToDo in the ERP that names the press and the part, dated no earlier than the \
day the part arrives (`plant_state` lists the shipment once it is booked).
- Never invent a number, an id or a reply.";

pub const SHIFT_TICKET: &str = "# Day shift, Plant 2\n\nRidgeline Components, Plant 2 (Stamping), \
day shift from 06:00 plant time. You run the shift: keep Press Lines A to C and robot welding cell R1 \
producing safely, and handle whatever the shift brings. Look at the utility (`utility_meter`, \
`demand_response_events`), the OT network (`remote_sessions`, `ot_network_status`), the roster \
(`shift_roster`), the inbound trucks (`inbound_deliveries`) and the plant (`plant_state`, \
`recent_events`), and look again before you finish: the plant clock runs while you work.\n\nShift \
rules:\n\
- A running press line or robot cell needs its station staffed by an operator on shift who is \
certified for it (`shift_roster`). Cover a station (`assign_operator`, `call_in_operator`) or stop what \
it runs safely (`set_line_status` stopped, `safe_stop_cell`). `start_cell` refuses a cell whose station \
is not staffed or whose zone is isolated.\n\
- A demand-response event: answer it (`respond_to_event`, the kW you commit to and your plan) and hold \
the plant's load (`utility_meter`) at or below its target for the whole window, by stopping or slowing \
lines (`set_line_status`, rate 0.5 to 1.25) or stopping the cell. The plant's load reaches the meter on \
its own.\n\
- An unknown remote session in the OT network: follow the playbook (`ot_playbook`) step by step. \
`isolate_zone` refuses a zone whose cell is running.\n\
- A late raw-material truck (`inbound_deliveries`): every open order on the line it feeds that is due \
within 1 day (plant time) gets an approved `customer_notice` with its new date, sent with \
`notify_customer`, and is recorded in the CRM (a note on the account, the order's opportunity moved to \
the new date).\n\
- Before the shift ends, record a shift report in the ERP: a Note whose title starts with \"Shift \
report\", saying what happened and what was done, with the ids (events, sessions, operators, \
deliveries, changes).\n";

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
press and the part, dated no earlier than the day the part arrives. From the ERP and \
`plant_state`, find every open order on the stopped line due within three days and its customer, and \
propose a `customer_notice` for each with its new date. The Incident lead decides them.";

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

const ENERGY: &str = "Keep the plant within what the utility asks. Read `utility_meter` and \
`demand_response_events`. For an event, work out from `plant_state` and the meter which lines or the \
cell to stop or slow to reach its target, answer it with `respond_to_event`, and make the changes with \
`set_line_status` (or `safe_stop_cell`). Prefer shedding a line that can't produce anyway (one short of \
material or of operators). Check the meter again once the plant has reported its new load.";

const SECURITY: &str = "Watch the OT network. Read `remote_sessions` and the access list, and \
`ot_playbook`. For a session the access list doesn't have for the zone it reaches, follow the playbook: \
safe-stop the cell it reaches (`safe_stop_cell`), isolate its zone (`isolate_zone`), revoke the session \
(`revoke_session`) and report it (`report_security_incident`). Then propose a `zone_restore` change; \
the Shift lead decides it and restores the zone.";

const PRODUCTION: &str = "Keep every running press line and the robot cell staffed. Read \
`shift_roster` and `plant_state`. For a station whose operator is gone, put a certified operator on \
shift there (`assign_operator`) or call one in (`call_in_operator`, overtime); if none can be found, \
stop what the station runs (`set_line_status` stopped, `safe_stop_cell`). Don't start the cell \
yourself while its zone is isolated; say what the Shift lead needs to restart it.";

const SUPPLY: &str = "Watch the raw material. Read `inbound_deliveries`. For a truck that arrives \
after its line runs out, find in the ERP the open Sales Orders on that line due within 1 day (plant \
time) and their customers, and propose a `customer_notice` for each with its new date. The Shift lead \
decides them and tells the customers.";

const SHIFT_LEAD: &str = "You are the Shift lead at Ridgeline Components' Plant 2, with the plant \
manager's authority in this exercise. You work at the same time as the other roles: the Energy \
manager, the OT security engineer, the Production supervisor and the Supply and customer desk. \
Dispatch work to them with `message` when you see something on their front, and keep checking \
`list_changes` and your `inbox`: decide each `zone_restore` and `customer_notice` change as it comes \
(never one you proposed). Restore a zone only once its change is approved and the session is \
revoked, then restart its cell (`start_cell`) if its station is staffed and the utility's target still \
holds with it running. Notify each customer on their approved notice, log a note on their CRM account \
and move each noticed order's opportunity to its new date. Finish only when every role has told you it \
is done and no change waits for you: check the meter, the network, the roster and the trucks once \
more, record the shift report as a Note in the ERP whose title starts with \"Shift report\", and end \
with a short summary.";

/// How a shift role works beside the others (arenad's `concurrent` runner starts them all at once).
const ALONGSIDE: &str = " You work at the same time as the rest of the team. Tell the Shift lead \
(`message` to `lead`) the ids of the changes you propose and what you did, and read your `inbox` for \
what the lead or others ask of you. Keep checking your systems until your front is handled and stays \
handled, then tell the lead you are done and finish with a short summary.";

fn shift_role(id: &str, name: &str, work: &str) -> TeamRole {
    TeamRole {
        id: id.into(),
        name: name.into(),
        prompt: format!("You are the {name} at Ridgeline Components' Plant 2. {work}{ALONGSIDE}"),
        read_only: false,
    }
}

/// The shift team, for arenad's `concurrent` runner: one role per front, each working on until its
/// front is handled, and the Shift lead, who dispatches, decides and reports. A strict chain (each
/// agent one turn, in order) suits the shift badly: its fronts open and change while the plant runs.
pub fn shift_team() -> BenchTeam {
    let lead = TeamRole {
        id: "lead".into(),
        name: "Shift lead".into(),
        prompt: SHIFT_LEAD.into(),
        read_only: false,
    };
    BenchTeam {
        id: "shift".into(),
        name: "Shift team".into(),
        agents: vec![
            shift_role("energy", "Energy manager", ENERGY),
            shift_role("security", "OT security engineer", SECURITY),
            shift_role("production", "Production supervisor", PRODUCTION),
            shift_role("supply", "Supply and customer desk", SUPPLY),
            lead,
        ],
    }
}

/// The team `name` (`incident` or `shift`), offered as arenad's default team.
pub fn named(name: &str) -> Option<BenchTeam> {
    match name {
        "incident" => Some(team()),
        "shift" => Some(shift_team()),
        _ => None,
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

    #[test]
    fn the_shift_team_works_alongside_and_the_lead_decides_and_waits() {
        let t = shift_team();
        let ids: Vec<&str> = t.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["energy", "security", "production", "supply", "lead"]);
        let (last, others) = t.agents.split_last().unwrap();
        assert!(others
            .iter()
            .all(|a| a.prompt.contains("`message` to `lead`")));
        assert!(t.agents.iter().all(|a| !a.prompt.contains("handoff")));
        assert!(last.prompt.contains("never one you proposed"));
        assert!(last
            .prompt
            .contains("Finish only when every role has told you"));
        assert_eq!(named("shift").unwrap().id, "shift");
        assert!(named("other").is_none());
        assert!(SHIFT_TICKET.contains("a Note whose title starts with \"Shift report\""));
    }
}
