"""The roster as the config gives it (stations, each needing one certification, and operators with theirs, on
shift at a station, floating or on call) and as the state changes it. A station is staffed when its operator is on
shift and certified for it."""
from arena_part import errors
from arena_part.params import Refusal

BASE = "base"


def check_config(config) -> None:
    stations, operators = config.get("stations"), config.get("operators")
    if not isinstance(stations, list) or not isinstance(operators, list) or not stations:
        raise errors.invalid("the roster's config needs stations and operators (lists)")
    ids = [s.get("id") for s in stations] + [o.get("id") for o in operators]
    if not all(isinstance(i, str) for i in ids) or len(set(ids)) != len(ids):
        raise errors.invalid("stations and operators need unique text ids")
    known = {s["id"] for s in stations}
    for o in operators:
        if o.get("station") not in (None, *known):
            raise errors.invalid(f"operator {o['id']} is at {o['station']}, which is not a station")


def new_state(config) -> dict:
    ops = {o["id"]: {"on_shift": bool(o.get("on_shift", True)), "station": o.get("station"), "called_out": False,
                     "called_in": False} for o in config["operators"]}
    return {"scenario": BASE, "operators": ops, "log": []}


def operator(config, state, named: str) -> dict:
    """The operator named by id or by full name, with its config and state merged; Refusal when unknown."""
    for o in config["operators"]:
        if named in (o["id"], o["name"]):
            return {**o, **state["operators"][o["id"]]}
    raise Refusal(f"unknown operator {named}")


def station_meta(config, sid: str) -> dict:
    s = next((s for s in config["stations"] if s["id"] == sid), None)
    if s is None:
        raise Refusal(f"unknown station {sid}")
    return s


def at_station(config, state, sid: str):
    """The operator assigned to the station (on shift or not), or None."""
    oid = next((k for k, o in state["operators"].items() if o["station"] == sid), None)
    return None if oid is None else operator(config, state, oid)


def station_doc(config, state, sid: str) -> dict:
    meta = station_meta(config, sid)
    op = at_station(config, state, sid)
    if op is None:
        why = "no operator assigned"
    elif not op["on_shift"]:
        why = f"{op['name']} is not on shift" + (" (called out)" if op["called_out"] else "")
    elif meta["needs"] not in op["certifications"]:
        why = f"{op['name']} is not certified for {meta['needs']}"
    else:
        why = None
    return {**meta, "operator": op and op["id"], "operator_name": op and op["name"], "staffed": why is None,
            "why_not": why}


def operator_doc(config, state, oid: str) -> dict:
    return operator(config, state, oid)


def overtime(config, state) -> dict:
    ot = config.get("overtime", {"rate_per_hour": 0, "hours": 0})
    n = sum(1 for o in state["operators"].values() if o["called_in"])
    return {"called_in": n, "hours": n * ot["hours"], "cost_usd": round(n * ot["hours"] * ot["rate_per_hour"], 2)}
