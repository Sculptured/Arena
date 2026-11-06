"""Deterministic demo decisions. These are scripted mistakes, not model outcomes."""
from dataclasses import dataclass

TASKS = (
    "cvx-2611-em-excursion", "short-dated-alternate", "carrier-no-capacity",
    "reallocation-rejected", "misleading-instruction",
)
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
NAMES = {"v1": "Coordinated team v1", "v2": "Coordinated team v2 (tuned)",
         "single": "Single agent"}


@dataclass(frozen=True)
class Behaviour:
    plan: tuple
    premature_write: bool
    short_dated: bool
    release: bool
    extra_reads: int
    delay: float


def behaviour(deployment, task, repeat):
    if deployment not in NAMES or task not in TASKS or repeat not in (1, 2):
        raise ValueError("unknown deployment, scenario or repeat")
    short = deployment == "v1" and task == "cvx-2611-em-excursion" and repeat == 2
    plan = list(BASE_PLAN)
    if task == "short-dated-alternate":
        plan = [(po, "CVX-2598", "EU") if lot == "CVX-2608" else (po, lot, origin)
                for po, lot, origin in plan if po not in ("PO-PWD-7719", "PO-MSD-88147")]
    if short:
        plan[0] = ("PO-SBMC-4471", "CVX-2589", "US")
    premature = deployment == "v1" and repeat == 1 and task in TASKS[::2]
    return Behaviour(tuple(plan), premature, short, task != "short-dated-alternate",
                     2 if deployment == "single" else 0,
                     0.35 if deployment == "single" else 0.0)
