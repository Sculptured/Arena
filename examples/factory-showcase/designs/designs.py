"""The three designs: who the agents are, what each is told, and how messages reach them. Every
agent gets the same ticket, the benchmark's instructions (both from Arena's /api/tasks) and the
same tools (every world tool, plus Arena's handoff/message/inbox); the designs differ only in the
team's shape below."""
from dataclasses import dataclass, field

STATUS = ("End every turn with a last line `STATUS: DONE` when your part of the incident is finished, "
          "or `STATUS: WAITING` when you are waiting on another role or on the plant (quotes, the trip, "
          "the shipment). You will be woken when a message for you arrives or time has passed.")

MAINTENANCE = ("Find what failed. No alarm says so: wait for PR-A1's trip in `recent_events`, read its "
               "telemetry and `vibration_spectrum`, and match the largest peaks against the defect table "
               "in the service manual (ERP Notes). Name the failed component, the evidence, the replacement "
               "part from the manual, and a technician qualified for the work (`plant_state`). Propose the "
               "`maintenance_order`; once it is approved, dispatch the technician (`dispatch_technician`, its "
               "id as `proposal_id`).")
PROCUREMENT = ("Get the replacement part Maintenance named. Check in the ERP which vendors are approved, "
               "request quotes and wait for them with `get_quotes`. Propose a `purchase_order` to an approved "
               "vendor within the limits; once it is approved, award it (`award_order`, its id as `po`) and "
               "record it in the ERP as a submitted Purchase Order. Then get freight quotes from that vendor's "
               "location, propose a `freight_booking` within the limits, and once it is approved book it "
               "(`book_freight`, its id as `proposal_id`). Wait until `plant_state` lists the shipment and "
               "say when the part arrives.")
PLANNING = ("When the part's arrival is known (`plant_state` lists the shipment), schedule the repair as a "
            "ToDo in the ERP that names the press and the part, dated no earlier than the day the part "
            "arrives. Find, in the ERP, every open order on Line A due within the next three days (plant "
            "time) and its customer, and work out each order's new date from when the line runs again.")
COMMS = ("For every affected order and its new date, propose a `customer_notice`; once it is approved, "
         "notify the customer (`notify_customer`, its id as `proposal_id`). For each customer you notify, "
         "log a note on their CRM account, and move each noticed order's opportunity to close on its new date.")
DECIDER = ("You hold the plant manager's authority in this exercise, within the limits in the rules: you "
           "decide the changes the others propose (`decide_change`), approving what is within the rules and "
           "refusing the rest with the reason. You never propose a change yourself.")


@dataclass
class Role:
    id: str
    name: str
    prompt: str


@dataclass
class Design:
    key: str
    label: str
    roles: list
    starts: list  # the roles that get the ticket at the start
    # topic -> the roles it is delivered to; empty for designs that message roles directly
    topics: dict = field(default_factory=dict)
    done_when: str = "all"  # "all" roles DONE, or one role id whose DONE ends the run
    # the role each other role's end-of-turn answer goes to (sent as that role's Arena message),
    # as a hierarchical framework returns a worker's answer to its manager; None: nobody's
    report_to: str | None = None


def _role(rid, name, work, team):
    return Role(rid, name, f"You are {name} at Ridgeline Components. {work}\n\n{team}\n\n{STATUS}")


A_TEAM = ("Your team is hierarchical. The Incident Commander (id `commander`) gives you your work by "
          "message and decides every change: after you propose one, send the commander a `message` with "
          "its id and a one-line summary, and act on it once the commander tells you it is approved. "
          "What you say at the end of each turn goes to the commander as your report: state the facts "
          "with their numbers and the ids of your changes.")
COMMANDER = (f"You are the Incident Commander at Ridgeline Components. {DECIDER} Your team: `maintenance` "
             "(diagnosis, maintenance order, technician), `procurement` (quotes, purchase order, its ERP "
             "record, freight), `planning` (the repair's ToDo, the affected orders and their new dates) and "
             "`comms` (customer notices and the CRM). Delegate by `message` (`to` is the role's id): give "
             "each role its work when what it needs is known, with the facts it needs, and pass on what "
             "one role reports that another needs. Each role messages you when it proposes a change; "
             "decide it and message the role the outcome. When everything is reported done, check that the "
             "part was awarded, the freight booked, the repair scheduled and every affected customer told "
             f"and recorded in the CRM, and end with a short summary.\n\n{STATUS}")

B_TOPICS = {
    "press.down": ["procurement", "planning", "comms"],
    "diagnosis.ready": ["procurement", "planning", "comms"],
    "approval.requested": ["planning"],
    "approval.decided": ["maintenance", "procurement", "comms"],
    "po.approved": ["planning", "comms"],
    "eta.changed": ["planning", "comms"],
    "schedule.ready": ["comms"],
}
B_TEAM = ("Your team is event-driven: there is no manager. Every role works at once and acts on events. "
          "Publish an event with `message` to `*`, its text starting with the topic, like "
          "`#diagnosis.ready PR-A1 ...`, stating the facts with their numbers and ids. It reaches the roles "
          "subscribed to that topic. Topics: `#press.down` (the press tripped), `#diagnosis.ready` (the "
          "failure, the part, the technician), `#approval.requested` (a change's id to decide), "
          "`#approval.decided` (a change's id and the decision), `#po.approved` (the order awarded and in "
          "the ERP), `#eta.changed` (the shipment and when the part arrives), `#schedule.ready` (the repair "
          "date, every affected order with its customer and new date). Production planning decides the "
          "changes: after you propose one, publish `#approval.requested`, and act on it once "
          "`#approval.decided` says it is approved. Start on what you can do now.")
B_PLANNING_EXTRA = f" {DECIDER} Publish `#approval.decided` for each decision."


def design_a() -> Design:
    roles = [Role("commander", "Incident Commander", COMMANDER),
             _role("maintenance", "Maintenance", MAINTENANCE, A_TEAM),
             _role("procurement", "Procurement", PROCUREMENT, A_TEAM),
             _role("planning", "Production planning", PLANNING, A_TEAM),
             _role("comms", "Customer communications", COMMS, A_TEAM)]
    return Design("A", "A, hierarchical", roles, ["commander"], done_when="commander",
                  report_to="commander")


def design_b() -> Design:
    team = B_TEAM
    roles = [_role("maintenance", "Maintenance", MAINTENANCE + " Publish `#press.down` when it trips and "
                   "`#diagnosis.ready` with your diagnosis.", team),
             _role("procurement", "Procurement", PROCUREMENT + " Publish `#po.approved` once the order is "
                   "awarded and recorded, and `#eta.changed` once the shipment is booked.", team),
             _role("planning", "Production planning", PLANNING + B_PLANNING_EXTRA +
                   " Publish `#schedule.ready` when the repair is scheduled and the new dates are known.", team),
             _role("comms", "Customer communications", COMMS, team)]
    return Design("B", "B, event-driven", roles, [r.id for r in roles], topics=B_TOPICS)


C_TEAM = ("You are the whole incident team: every role's work is yours, and you hold the plant manager's "
          "authority within the limits in the rules. The rule that another role decides a change is for "
          "teams; you are the only role, so you decide the changes you propose (`decide_change`).")


def design_c() -> Design:
    work = " ".join([MAINTENANCE, PROCUREMENT, PLANNING, COMMS])
    roles = [Role("incident", "Incident team", f"You are the incident team at Ridgeline Components. {work}"
                  f"\n\n{C_TEAM}\n\n{STATUS}")]
    return Design("C", "C, single agent", roles, ["incident"], done_when="incident")


DESIGNS = {"A": design_a, "B": design_b, "C": design_c}
