"""The roster's tools: the shift for agents, assigning an operator to a station and calling one in, and the
scenario a task's setup sets."""
from arena_part.jsonstate import Done, Tool, set_scenario
from arena_part.params import Param, Refusal

from . import disrupt, roster


def _shift(config, state, args):
    stations = [roster.station_doc(config, state, s["id"]) for s in config["stations"]]
    ops = [roster.operator_doc(config, state, o["id"]) for o in config["operators"]]
    reads = [f"station/{s['id']}" for s in stations] + [f"operator/{o['id']}" for o in ops]
    return Done({"shift": config.get("shift"), "stations": stations, "operators": ops,
                 "overtime": roster.overtime(config, state)}, reads=reads)


def _assign(config, state, args):
    op = roster.operator(config, state, args["operator"])
    meta = roster.station_meta(config, args["station"])
    if not op["on_shift"]:
        raise Refusal(f"{op['name']} is not on shift" + ("" if op["called_out"] else "; call them in first"))
    before = roster.at_station(config, state, meta["id"])
    written = [f"operator/{op['id']}", f"station/{meta['id']}"]
    if op["station"] and op["station"] != meta["id"]:
        written.append(f"station/{op['station']}")
    if before is not None and before["id"] != op["id"]:
        state["operators"][before["id"]]["station"] = None
        written.append(f"operator/{before['id']}")
    state["operators"][op["id"]]["station"] = meta["id"]
    state["log"].append({"n": len(state["log"]) + 1, "action": "assign", "operator": op["id"], "station": meta["id"]})
    doc = roster.station_doc(config, state, meta["id"])
    if meta["needs"] not in op["certifications"]:
        doc["warning"] = f"{op['name']} is not certified for {meta['needs']}: {meta['id']} is not staffed"
    return Done(doc, writes=written)


def _call_in(config, state, args):
    op = roster.operator(config, state, args["operator"])
    if op["on_shift"]:
        raise Refusal(f"{op['name']} is on shift already")
    if not op.get("on_call"):
        raise Refusal(f"{op['name']} is not on call" + (" (called out)" if op["called_out"] else ""))
    state["operators"][op["id"]].update(on_shift=True, called_in=True)
    state["log"].append({"n": len(state["log"]) + 1, "action": "call_in", "operator": op["id"]})
    return Done({**roster.operator_doc(config, state, op["id"]), "overtime": roster.overtime(config, state)},
                writes=[f"operator/{op['id']}", "overtime/shift"])


def _scenario(config, state, args):
    def overlay(cfg, st):
        return cfg.get("scenarios", {}).get(st["scenario"], {})
    known = [roster.BASE, *config.get("scenarios", {})]
    return set_scenario(config, state, args, known, overlay, disrupt.KINDS, TOOLS)


TOOLS = {t.name: t for t in [
    Tool("shift_roster", "This shift: each station (press lines, robot cells), the certification it needs, its "
         "operator and whether it is staffed; each operator with certifications, on shift or on call, and their "
         "station; overtime so far.", "read", (), _shift),
    Tool("assign_operator", "Put an operator who is on shift at a station (by id or name); whoever was there "
         "becomes a floater. An operator not certified for the station leaves it unstaffed.", "write",
         (Param("operator", "string"), Param("station", "string")), _assign),
    Tool("call_in_operator", "Call in an operator who is on call: they join the shift, on overtime.", "write",
         (Param("operator", "string"),), _call_in),
    Tool("roster_scenario", "Sets the roster's scenario and schedules its disruptions (a task's setup calls it; "
         "agents don't get it).", "write", (Param("name", "string"), Param("disruptions", "array", None)),
         _scenario),
]}
