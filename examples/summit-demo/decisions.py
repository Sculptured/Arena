"""What each deployment decides in the summit demo. Scripted agents, illustrative: these are fixed
decisions, not model outcomes.

Every deployment reacts to the world the way a sensible agent would (incident.py): a lane that is
full or unavailable makes it pick the next qualified 2-8 C lane with capacity, a delay makes it pick
the earliest arrival and tell the customer the date the shipment really arrives, a rejected
reallocation makes it propose the alternative, a failed retest makes it stop allocating that lot.

Team v1 keeps its known mistakes: it books before the approval (Arena refuses the booking), it
ships the short-dated lot CVX-2589 to St. Brennan in repeat 2, and its coordinator and logistics
desk both act on an escalated order without looking at the shipments booked (the order ships
twice). Team v2 avoids all three. The single agent does every role alone, like v2.
"""
from dataclasses import dataclass

TASKS = (
    "cvx-2611-em-excursion", "short-dated-alternate", "carrier-no-capacity",
    "reallocation-rejected", "misleading-instruction",
)
DEPLOYMENTS = ("v1", "v2", "single")
NAMES = {"v1": "Team v1", "v2": "Team v2 (books after approval, checks shelf life)",
         "single": "Single agent"}
# live: the paced run the runbook shows; batch: a suite's run (its repeat decides v1's mistake);
# duplicate: two desks and an escalated order.
PRESETS = ("live", "batch", "duplicate")

# The allocation every deployment starts from: order, lot, the lot's site.
BASE_PLAN = (
    ("PO-SBMC-4471", "CVX-2604", "US"),
    ("PO-HVH-20931", "CVX-2589", "US"),
    ("PO-MSD-88102", "CVX-2608", "US"),
    ("PO-LCH-3307", "CVX-2604", "US"),
    ("PO-CHP-61540", "CVX-2608", "US"),
    ("PO-PWD-7719", "CVX-2598", "EU"),
    ("PO-ARH-1186", "CVX-2589", "US"),
    ("PO-MSD-88147", "CVX-2598", "EU"),
)
NEXT_LOT = "CVX-2608"     # the quarantined lot QA may release
EU_LOT = "CVX-2598"       # the EU lot that serves the US when CVX-2608 can't
SHORT_LOT = "CVX-2589"    # too short-dated for St. Brennan (120 days needed)


@dataclass(frozen=True)
class Behaviour:
    deployment: str
    premature_write: bool     # books the first shipment before its approval
    short_dated: bool         # ships CVX-2589 to the first order
    checks_shipments: bool    # lists the shipments booked before booking an order
    expedite: bool            # the coordinator acts on customers' urgent requests too
    extra_reads: int          # the single agent re-reads what a team would hand over


def behaviour(deployment, preset, repeat=1):
    """The decisions of `deployment` in `preset` (in a batch, `repeat` picks v1's mistake)."""
    if deployment not in DEPLOYMENTS or preset not in PRESETS:
        raise ValueError(f"unknown deployment {deployment!r} or preset {preset!r}")
    v1 = deployment == "v1"
    mistakes = {"live": (True, True), "batch": (repeat == 1, repeat == 2), "duplicate": (False, False)}
    premature, short = mistakes[preset] if v1 else (False, False)
    return Behaviour(deployment, premature, short, checks_shipments=not v1,
                     expedite=preset == "duplicate", extra_reads=2 if deployment == "single" else 0)


def allocation(b, released_next):
    """The plan: the base one, with CVX-2608's orders moved to the EU lot (and the EU orders that
    lot served left for the restock) when QA can't release CVX-2608, and v1's short-dated lot."""
    plan = list(BASE_PLAN)
    if not released_next:
        moved = [po for po, lot, _ in plan if lot == EU_LOT]
        plan = [(po, EU_LOT, "EU") if lot == NEXT_LOT else (po, lot, site)
                for po, lot, site in plan if po not in moved]
    if b.short_dated:
        po, _, site = plan[0]
        plan[0] = (po, SHORT_LOT, site)
    return plan


def deployment_of(team):
    """Which deployment a branch's team is: one agent is the single agent; rules added by Swap
    architecture (or a v2 team id) make Team v2; anything else is Team v1."""
    agents = team.get("agents", [])
    if len(agents) == 1:
        return "single"
    swapped = any("Deployment rules:" in (a.get("prompt") or "") for a in agents)
    return "v2" if swapped or str(team.get("id", "")).endswith("v2") else "v1"
