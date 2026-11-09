"""KPIs: each replication's figures from a simulator's raw result, and their mean with a 95%
confidence interval over the replications (Student's t). Also the outcome constraints a config can
only be judged on after it ran (Line 1's commitment, Line 2's lead time) and the objective.

OEE here is good units × the line's ideal cycle time over all staffed time: planned maintenance,
changeovers, failures, waiting for a technician, blocking and starving all count as losses.
"""
import math
import statistics

from .plant import headcount

HOUR = 3600.0
# Two-sided 95% Student's t quantiles by degrees of freedom; 1.96 beyond.
T95 = [12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
       2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
       2.052, 2.048, 2.045, 2.042]

# KPI name -> (label, unit). The order is the order they are reported in.
KPIS = {
    "line2.oee": ("Line 2 OEE", "%"),
    "line2.quality": ("Line 2 first-pass quality", "%"),
    "line2.good_per_week": ("Line 2 good units per week", "units"),
    "line2.wip": ("Line 2 mean WIP", "units"),
    "line2.lead_time_h": ("Line 2 lead time (order to packed)", "h"),
    "line1.oee": ("Line 1 OEE", "%"),
    "line1.good_per_week": ("Line 1 good units per week", "units"),
    "cost_per_week_usd": ("Operating cost per week", "USD"),
}


def _line(raw: dict, line_id: str) -> dict:
    return next(x for x in raw["lines"] if x["id"] == line_id)


def _line_kpis(spec: dict, r: dict, measured: float) -> dict:
    made = r["good"] + r["scrap"]
    out = {f"{spec['id']}.oee": 100.0 * r["good"] * spec["ideal_cycle_s"] / measured,
           f"{spec['id']}.good_per_week": float(r["good"])}
    if len(spec["variants"]) > 1:
        flow = r["flow_mean_s"] or 0.0
        wait = (r["campaign_cycle_s"] or 0.0) / 2  # an order waits on average half a campaign cycle
        out.update({f"{spec['id']}.quality": 100.0 * r["good"] / made if made else 0.0,
                    f"{spec['id']}.wip": r["wip_mean"],
                    f"{spec['id']}.lead_time_h": (flow + wait) / HOUR})
    return out


def _cost(plant: dict, config: dict, raw: dict) -> float:
    week = plant["week"]
    hours = week["shifts"] * week["shift_hours"]
    m = plant["maintenance"]
    cost = headcount(config) * hours * plant["labour_usd_per_h"]
    for spec in plant["lines"]:
        r = _line(raw, spec["id"])
        failures = sum(s["failures"] for s in r["stations"])
        cycle_stock = spec["batch_size"] / 2 if len(spec["variants"]) > 1 else 0
        cost += failures * m["repair_usd"] + r["pm_s"] / HOUR * m["pm_parts_usd_per_h"]
        cost += r["scrap"] * spec["scrap_usd"]
        cost += (r["wip_mean"] + cycle_stock) * hours * plant["holding_usd_per_unit_h"]
    return cost


def replication(plant: dict, config: dict, raw: dict) -> dict:
    """One replication's KPIs. `plant` is the plant with `config` applied."""
    out = {}
    for spec in plant["lines"]:
        out.update(_line_kpis(spec, _line(raw, spec["id"]), raw["measured_s"]))
    out["cost_per_week_usd"] = _cost(plant, config, raw)
    return {k: out[k] for k in KPIS if k in out}


def summary(values: list) -> dict:
    """Mean and the 95% confidence half-width (None with one replication)."""
    mean = statistics.fmean(values)
    if len(values) < 2:
        return {"mean": round(mean, 3), "ci95": None}
    t = T95[len(values) - 2] if len(values) - 2 < len(T95) else 1.96
    half = t * statistics.stdev(values) / math.sqrt(len(values))
    return {"mean": round(mean, 3), "ci95": round(half, 3)}


def aggregate(reps: list) -> dict:
    """Each KPI over the replications, with its label and unit."""
    return {k: {**summary([r[k] for r in reps]), "label": KPIS[k][0], "unit": KPIS[k][1]}
            for k in KPIS if k in reps[0]}


def stations(plant: dict, raws: list) -> dict:
    """Per line, each station's share of time in each state (%), and its failures per week."""
    out = {}
    for spec in plant["lines"]:
        rows = []
        for i, st in enumerate(spec["stations"]):
            per = [_line(raw, spec["id"])["stations"][i] for raw in raws]
            share = {k: round(100 * statistics.fmean(p["share"][k] for p in per), 1) for k in per[0]["share"]}
            rows.append({"station": st["name"], **share,
                         "failures_per_week": round(statistics.fmean(p["failures"] for p in per), 1)})
        out[spec["id"]] = rows
    return out


def outcome_constraints(plant: dict, kpis: dict) -> list:
    """The limits a config is judged on once simulated: each with its value and whether it held."""
    out = []
    for spec in plant["lines"]:
        lid = spec["id"]
        if "min_good_per_week" in spec:
            v = kpis[f"{lid}.good_per_week"]["mean"]
            out.append({"name": f"{lid}.good_per_week", "rule": f">= {spec['min_good_per_week']}",
                        "value": v, "met": v >= spec["min_good_per_week"],
                        "why": f"{spec['name']} is committed to {spec['min_good_per_week']} good units a week"})
        if "max_lead_time_h" in spec:
            v = kpis[f"{lid}.lead_time_h"]["mean"]
            out.append({"name": f"{lid}.lead_time_h", "rule": f"<= {spec['max_lead_time_h']}",
                        "value": v, "met": v <= spec["max_lead_time_h"],
                        "why": f"{spec['name']} customers are promised {spec['max_lead_time_h']} h"})
    return out
