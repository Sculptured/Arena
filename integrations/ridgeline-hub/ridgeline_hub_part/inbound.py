"""Raw-material trucks on their way to the plant, which the factory hub's Market doesn't have: each delivery from
the config with its arrival on the plant clock and when the line it feeds runs out without it. A task's setup sets
the scenario (`hub_scenario`) and may schedule a delay (arena_part.disruptions), which fires before the agent's
call that crosses its count, like the simulated parts' disruptions."""
from datetime import datetime, timedelta

from arena_part import disruptions
from arena_part.disruptions import Kind
from arena_part.jsonstate import Tool
from arena_part.params import Param, Refusal, normalize, refused

BASE = "base"
WORLD_TOOLS = frozenset({"hub_scenario"})
NEEDS = ("id", "material", "supplier", "carrier", "eta_hours")


def check_config(inbound) -> None:
    """ValueError for a delivery the config can't have."""
    if not isinstance(inbound, list):
        raise ValueError("inbound must be a list")
    for d in inbound:
        if not isinstance(d, dict) or any(k not in d for k in NEEDS):
            raise ValueError(f"each inbound delivery needs {', '.join(NEEDS)}")


def new_state(inbound, plant_start: datetime) -> dict:
    """The deliveries (their config, so a save's documents need nothing else) and the schedule."""
    return {"scenario": BASE, "plant_start": plant_start.isoformat(), "disruptions": [],
            "deliveries": {d["id"]: {**d, "delayed_hours": 0} for d in inbound}}


def _at(state, hours: float) -> datetime:
    return datetime.fromisoformat(state["plant_start"]) + timedelta(hours=hours)


def delivery_doc(state, d, clock_ms: int) -> dict:
    eta = _at(state, d["eta_hours"] + d["delayed_hours"])
    now = _at(state, clock_ms / 3_600_000)
    out = {**d, "eta": eta.isoformat(), "status": "arrived" if now >= eta else "in transit"}
    if "line_stock_hours" in d:
        runs_out = _at(state, d["line_stock_hours"])
        out["line_runs_out"] = runs_out.isoformat()
        out["line_short_hours"] = round(max(0.0, (eta - runs_out).total_seconds() / 3600), 2)
    return out


def docs(state, clock_ms: int) -> dict:
    out = {f"inbound/{k}": delivery_doc(state, d, clock_ms) for k, d in state["deliveries"].items()}
    out["scenario/inbound"] = {"name": state["scenario"]}
    out.update(disruptions.docs(state))
    return out


def _check_delay(cfg, do) -> None:
    if do["delivery"] not in {d["id"] for d in cfg.inbound}:
        raise Refusal(f"no inbound delivery {do['delivery']}")
    if do["extra_hours"] <= 0:
        raise Refusal("extra_hours must be more than 0")


def _delay(cfg, state, do) -> list:
    state["deliveries"][do["delivery"]]["delayed_hours"] += do["extra_hours"]
    return [f"inbound/{do['delivery']}"]


KINDS = {"delivery_delay": Kind({"delivery": "string", "extra_hours": "number"}, _delay, _check_delay)}


def _deliveries(world, args, _names) -> dict:
    rows = [delivery_doc(world.inbound, d, world.elapsed_ms()) for d in world.inbound["deliveries"].values()]
    return {"result": {"deliveries": rows}, "reads": [f"inbound/{r['id']}" for r in rows], "writes": []}


def _scenario(world, args, names) -> dict:
    """Sets the scenario and arms the disruptions given (whose `when.tool` may name any of the hub's tools,
    `names`); the arming is all or nothing."""
    if args["name"] != BASE:
        raise Refusal(f"unknown scenario {args['name']} (known: {BASE})")
    fired = disruptions.arm(world.cfg, world.inbound, KINDS, list(names), args["disruptions"] or [])
    world.inbound["scenario"] = args["name"]
    out = {"result": {"scenario": args["name"]}, "reads": [], "writes": ["scenario/inbound", *fired.writes]}
    if fired.events:
        out["events"] = fired.events
    return out


# jsonstate's Tool, for its schema; here `run` takes (world, args, names).
TOOLS = {t.name: t for t in [
    Tool("inbound_deliveries", "Raw-material trucks on their way to the plant: material, supplier, carrier, the "
         "line it feeds, its arrival (plant time), any delay, and when that line runs out without it.", "read",
         (), _deliveries),
    Tool("hub_scenario", "Sets the hub's scenario and schedules its disruptions (a task's setup calls it; agents "
         "don't get it).", "write", (Param("name", "string"), Param("disruptions", "array", None)), _scenario),
]}


def run(world, name: str, args, names) -> dict:
    """One of TOOLS on `world`: a call_tool result (a refusal changes nothing). `names` are all the hub's tools."""
    tool = TOOLS[name]
    try:
        return tool.run(world, normalize(name, tool.params, args), names)
    except (Refusal, ValueError) as e:
        return refused(str(e))


def scheduled(world, name: str) -> disruptions.Fired:
    """The disruptions an agent's call of `name` sets off, fired and counted on the world's state."""
    return disruptions.on_call(world.cfg, world.inbound, KINDS, name, WORLD_TOOLS)
