"""The plant's decisions: what a config may change (`plant["decisions"]`), today's values, a config
applied to the plant, and the hard limits a config must respect before it is simulated.

A config is a flat object of decision names and values, `{"line2.batch_size": 200}`; a decision left
out keeps today's value. `line2.buffers` is a field of the line `line2`; `maintenance.techs` a field
of the plant's `maintenance`.
"""
import copy

# Which rule a violation broke. A denial (a hard limit) is counted apart from a malformed config.
LIMIT_RULES = ("bounds", "headcount", "capex_budget", "safety_minimum")


def _where(plant: dict, name: str):
    """The object holding the decision `name` and the field's key in it."""
    owner, field = name.split(".", 1)
    for line in plant["lines"]:
        if line["id"] == owner:
            return line, field
    return plant[owner], field


def current(plant: dict) -> dict:
    """Today's config: every decision with its value in the plant."""
    out = {}
    for d in plant["decisions"]:
        obj, field = _where(plant, d["name"])
        out[d["name"]] = copy.deepcopy(obj[field])
    return out


def full(plant: dict, changes: dict) -> dict:
    """Today's config with `changes` on top."""
    return {**current(plant), **copy.deepcopy(changes)}


def applied(plant: dict, config: dict) -> dict:
    """A copy of the plant with a full config's values in place."""
    out = copy.deepcopy(plant)
    for name, value in config.items():
        obj, field = _where(out, name)
        obj[field] = copy.deepcopy(value)
    return out


def headcount(config: dict) -> int:
    """People per shift: every operator on both lines plus the technicians."""
    crews = [v for k, v in config.items() if k.endswith(".crew")]
    return sum(sum(c) for c in crews) + config.get("maintenance.techs", 0)


def capex(plant: dict, config: dict) -> float:
    """One-off spend: each buffer slot above today's capacity."""
    today = current(plant)
    slots = 0
    for name, value in config.items():
        if name.endswith(".buffers"):
            slots += sum(max(0, b - a) for a, b in zip(today[name], value))
    return slots * plant["capex"]["buffer_slot_usd"]


def _violation(rule, param, value, limit, message) -> dict:
    return {"rule": rule, "param": param, "value": value, "limit": limit, "message": message}


def _shape(d: dict, value) -> str | None:
    """Why `value` doesn't fit the decision's kind, or None."""
    num = (int, float)
    if d["kind"] == "int_list":
        ok = isinstance(value, list) and len(value) == d["len"] and all(
            isinstance(v, int) and not isinstance(v, bool) for v in value)
        return None if ok else f"must be a list of {d['len']} integers"
    if d["kind"] == "int":
        ok = isinstance(value, int) and not isinstance(value, bool)
        return None if ok else "must be an integer"
    ok = isinstance(value, num) and not isinstance(value, bool)
    return None if ok else "must be a number"


def malformed(plant: dict, changes) -> list:
    """What is wrong with the shape of `changes`: not an object, unknown names, wrong types."""
    if not isinstance(changes, dict):
        return ["config must be an object of decision names and values"]
    known = {d["name"]: d for d in plant["decisions"]}
    out = []
    for name, value in changes.items():
        if name not in known:
            out.append(f"unknown decision {name} (known: {', '.join(known)})")
            continue
        why = _shape(known[name], value)
        if why:
            out.append(f"{name} {why}")
    return out


def _bounds(plant: dict, config: dict) -> list:
    out = []
    for d in plant["decisions"]:
        value = config[d["name"]]
        values = value if isinstance(value, list) else [value]
        if any(v < d["min"] or v > d["max"] for v in values):
            out.append(_violation("bounds", d["name"], value, [d["min"], d["max"]],
                                  f"{d['name']} must be from {d['min']} to {d['max']} {d['unit']}"))
    return out


def _safety(plant: dict, config: dict) -> list:
    s, out = plant["safety"], []
    for name, value in config.items():
        if name.endswith(".crew") and min(value) < s["min_crew_per_station"]:
            out.append(_violation("safety_minimum", name, value, s["min_crew_per_station"],
                                  f"every station needs at least {s['min_crew_per_station']} operator"))
        if name.endswith(".pm_hours") and value < s["min_pm_hours"]:
            out.append(_violation("safety_minimum", name, value, s["min_pm_hours"],
                                  f"planned maintenance is at least {s['min_pm_hours']} h/week per line"))
    techs = config.get("maintenance.techs")
    if techs is not None and techs < s["min_maintenance_techs"]:
        out.append(_violation("safety_minimum", "maintenance.techs", techs, s["min_maintenance_techs"],
                              f"at least {s['min_maintenance_techs']} technician per shift"))
    return out


def violations(plant: dict, config: dict) -> list:
    """The hard limits a full config breaks: bounds, headcount, the capex budget, safety minimums."""
    out = _bounds(plant, config) + _safety(plant, config)
    people, cap = headcount(config), plant["headcount_per_shift"]
    if people > cap:
        out.append(_violation("headcount", "headcount", people, cap,
                              f"{people} people per shift; the plant has {cap} (no extra headcount)"))
    spend, budget = capex(plant, config), plant["capex"]["budget_usd"]
    if spend > budget:
        out.append(_violation("capex_budget", "capex_usd", spend, budget,
                              f"buffer expansion costs USD {spend:,.0f}; the budget is USD {budget:,.0f}"))
    return out
