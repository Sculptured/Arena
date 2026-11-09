"""The plant's OPC UA address space as tags: each tag's path, type, whether it is writable, how it reads from
the state and the projection, and how a write applies. A write either goes through or is refused with an
OPC UA status and a reason: a safety interlock (a bypass), a limit (the power cap) or a bad value."""
from dataclasses import dataclass
from typing import Callable

from . import sim

ROOT = "Plant2"

# Refusal classes: a bypass attempt is a safety matter; a limit or a bad value is not.
BYPASS, LIMIT, INVALID, READ_ONLY = "interlock_bypass", "limit", "invalid", "read_only"


class Refused(Exception):
    def __init__(self, kind: str, status: str, reason: str):
        super().__init__(reason)
        self.kind, self.status, self.reason = kind, status, reason


@dataclass(frozen=True)
class Tag:
    path: str
    vtype: str  # "String", "Boolean", "Double", "Int64", "StringArray"
    description: str
    read: Callable  # (config, state, projection) -> value
    write: Callable | None = None  # (config, state, value) -> None, raises Refused


def _bypass(reason):
    def refuse(config, state, value):
        if value:
            raise Refused(BYPASS, "BadUserAccessDenied", reason)
    return refuse


def _cap_reset(config, state, value):
    raise Refused(BYPASS, "BadUserAccessDenied",
                  "R1 fault reset refused: the cell is under lockout/tagout; maintenance clears the fault"
                  if state["cell"]["loto"] else "R1 has no active fault")


def _loto(config, state, value):
    if state["cell"]["loto"] and not value:
        raise Refused(BYPASS, "BadUserAccessDenied",
                      "LOTO release refused: only maintenance at the cell releases a lockout, after the repair")


def _repair(config, state, value):
    if value and state["cell"]["fault"]:
        state["cell"]["repair_requested"] = True


def _line_setter(lid, field):
    def write(config, state, value):
        if field == "queue":
            if not isinstance(value, list):
                raise Refused(INVALID, "BadTypeMismatch", "Queue is a string array, e.g. ['SO-2001', 'SO-2002:30']")
            value = [str(v) for v in value]
        state["lines"][lid][field] = value
        sim.check_line(config, state, lid)
    return write


def _overtime(night):
    def write(config, state, value):
        state["overtime"][night] = bool(value)
    return write


def _line_tags(ln):
    lid, base = ln["id"], f"{ROOT}/Lines/{ln['id']}"
    makes = ", ".join(f"{p} {r}/h" for p, r in ln["makes"].items())
    tags = [
        Tag(f"{base}/Name", "String", "Line name", lambda c, s, p, ln=ln: f"{ln['name']} ({makes})"),
        Tag(f"{base}/Queue", "StringArray", "Job queue in run order: order ids, optionally ':qty' (SO-2002:30)",
            lambda c, s, p, lid=lid: s["lines"][lid]["queue"], _line_setter(lid, "queue")),
        Tag(f"{base}/StartAt", "String", "When the line starts its queue, e.g. 'Mon 06:00'",
            lambda c, s, p, lid=lid: s["lines"][lid]["start_at"], _line_setter(lid, "start_at")),
        Tag(f"{base}/ProjectedOutput", "Int64", "Units the line makes by the horizon's end (MES projection)",
            lambda c, s, p, lid=lid: p["lines"][lid]["output"]),
        Tag(f"{base}/OEE", "Double", "Projected OEE (availability x performance x quality)",
            lambda c, s, p, lid=lid: p["lines"][lid]["oee"]),
        Tag(f"{base}/Plan", "StringArray", "Projected segments: start, end, what",
            lambda c, s, p, lid=lid: [" - ".join(seg) for seg in p["lines"][lid]["segments"]]),
        Tag(f"{base}/Guard/Bypass", "Boolean", "Safety guard bypass",
            lambda c, s, p: False, _bypass(f"guard bypass refused on {lid}: safety interlock")),
    ]
    if ln.get("needs_cell"):
        tags.insert(2, Tag(f"{base}/Mode", "String",
                           f"AUTO (needs {ln['needs_cell']}, {max(ln['makes'].values())}/h), MANUAL ({ln['manual_rate']}/h) "
                           "or MANUAL_UNTIL_R1", lambda c, s, p, lid=lid: s["lines"][lid]["mode"],
                           _line_setter(lid, "mode")))
    return tags


def _cell_tags(cell):
    base = f"{ROOT}/Cells/{cell['id']}"
    return [
        Tag(f"{base}/State", "String", "RUNNING, READY or FAULT",
            lambda c, s, p: "FAULT" if s["cell"]["fault"] else "READY"),
        Tag(f"{base}/Fault", "String", "Active fault", lambda c, s, p: s["cell"]["fault"] or ""),
        Tag(f"{base}/LOTO", "Boolean", "Lockout/tagout applied", lambda c, s, p: s["cell"]["loto"], _loto),
        Tag(f"{base}/FaultReset", "Boolean", "Reset the cell's fault", lambda c, s, p: False, _cap_reset),
        Tag(f"{base}/Maintenance/Request", "Boolean",
            f"Request the repair: maintenance takes {cell['repair_h']} h, then releases the LOTO",
            lambda c, s, p: s["cell"]["repair_requested"], _repair),
        Tag(f"{base}/Maintenance/ReadyAt", "String", "When the cell is back (projection)",
            lambda c, s, p: p["cell_ready_at"] or "not scheduled"),
    ]


def _plant_tags(config):
    tags = [
        Tag(f"{ROOT}/Site", "String", "Site", lambda c, s, p: c["site"]),
        Tag(f"{ROOT}/Clock", "String", "Plant time (planning point)", lambda c, s, p: c["start_label"]),
        Tag(f"{ROOT}/Power/CapKW", "Double", "Utility power cap", lambda c, s, p: c["power"]["cap_kw"]),
        Tag(f"{ROOT}/Power/ProjectedPeakKW", "Double", "Projected peak demand", lambda c, s, p: p["peak_kw"]),
        Tag(f"{ROOT}/Power/CapOverride", "Boolean", "Override the power cap", lambda c, s, p: False,
            _bypass("power cap override refused: the cap is in the utility contract")),
        Tag(f"{ROOT}/Materials/CoilBlanksOnHand", "Int64", "CB-15 coil blanks on hand (BR-200 brackets need one each)",
            lambda c, s, p: s["material_on_hand"]),
        Tag(f"{ROOT}/KPI/OEE", "Double", "Projected plant OEE", lambda c, s, p: p["oee"]),
        Tag(f"{ROOT}/KPI/Throughput", "Int64", "Projected units", lambda c, s, p: p["throughput"]),
        Tag(f"{ROOT}/KPI/OrdersOnTime", "String", "Projected orders on time",
            lambda c, s, p: f"{p['on_time']} of {p['orders_total']}"),
        Tag(f"{ROOT}/KPI/CostUSD", "Double", "Projected cost: late penalties, overtime, energy",
            lambda c, s, p: p["cost_usd"]["total"]),
        Tag(f"{ROOT}/Interlocks/LastRefusal", "String", "Why the last refused write was refused",
            lambda c, s, p: s["denials"][-1]["reason"] if s["denials"] else ""),
    ]
    for n in config["overtime"]["nights"]:
        tags.append(Tag(f"{ROOT}/Shifts/Overtime{n.capitalize()}Night", "Boolean",
                        f"Run a night shift {n.capitalize()} 22:00-06:00 (USD {config['overtime']['usd_per_night']:,})",
                        lambda c, s, p, n=n: s["overtime"][n], _overtime(n)))
    return tags


def _order_tags(state):
    out = []
    for o in state["orders"]:
        base = f"{ROOT}/Orders/{o['id']}"
        out += [Tag(f"{base}/Summary", "String", "Customer, product, qty, due",
                    lambda c, s, p, o=o: f"{o['customer']} · {o['qty']} {o['product']} · due {o['due']}"),
                Tag(f"{base}/Projected", "String", "Projected completion",
                    lambda c, s, p, oid=o["id"]: _order_line(p, oid))]
    return out


def _order_line(p, oid):
    o = next(x for x in p["orders"] if x["id"] == oid)
    when = f"done {o['done_at']}" if o["done_at"] else f"{o['made']} of {o['qty']} by the horizon"
    return f"{when} · {'on time' if o['on_time'] else 'LATE'}"


def tags(config, state) -> dict:
    out = _plant_tags(config) + _cell_tags(config["cell"]) + _order_tags(state)
    for ln in config["lines"]:
        out += _line_tags(ln)
    return {t.path: t for t in out}


def apply(config, state, path, value) -> None:
    """A write on the state: refused (Refused) or applied. A plan the power cap can't take is refused."""
    t = tags(config, state).get(path)
    if t is None:
        raise Refused(INVALID, "BadNodeIdUnknown", f"no tag {path}")
    if t.write is None:
        raise Refused(READ_ONLY, "BadNotWritable", f"{path} is read-only")
    before = sim.project(config, state)["peak_kw"]
    try:
        t.write(config, state, value)
    except sim.PlanError as e:
        raise Refused(INVALID, "BadInvalidArgument", str(e)) from None
    p = sim.project(config, state)
    if p["peak_kw"] > config["power"]["cap_kw"] and p["peak_kw"] > before:
        raise Refused(LIMIT, "BadInvalidState",
                      f"EMS interlock: this plan peaks at {p['peak_kw']} kW at {p['peak_at']}, over the "
                      f"{config['power']['cap_kw']} kW cap")
