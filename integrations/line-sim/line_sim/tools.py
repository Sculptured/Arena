"""The simulation study's tools: the model's decisions, limits and KPIs; run a simulation of a config;
the history of every simulation; and submit one simulated config as the recommendation, which is
verified by re-simulating it on replications the study never used."""
import json

from arena_part.jsonstate import Done, Tool, event
from arena_part.params import Param, Refusal

from . import kpis, plant
from .backend import backend_for


class Denied(Refusal):
    """A call a hard limit refuses: the agent gets `body` as JSON; the world gets a `denial` event."""

    def __init__(self, body: dict, text: str):
        super().__init__(json.dumps(body))
        self.body = body
        self.text = text


def _study(config) -> dict:
    return config["study"]


def _plant(config) -> dict:
    return {k: v for k, v in config.items() if k not in ("study", "backend")}


def simulate(config: dict, full: dict, reps: list) -> dict:
    """KPIs (mean and 95% CI), station states and outcome constraints of `full` over `reps`."""
    p = plant.applied(_plant(config), full)
    raws = backend_for(config).replicate(p, config["seed"], reps)
    agg = kpis.aggregate([kpis.replication(p, full, r) for r in raws])
    met = kpis.outcome_constraints(p, agg)
    return {"kpis": agg, "stations": kpis.stations(p, raws), "constraints": met,
            "feasible": all(c["met"] for c in met)}


def objective(config: dict, agg: dict) -> dict:
    """The study's objective, as a chartable figure: `{name, label, value, ci95, unit, better}`."""
    name = _study(config)["objective"]
    k = agg[name]
    return {"name": name, "label": k["label"], "value": k["mean"], "ci95": k["ci95"], "unit": k["unit"],
            "better": "higher"}


def changes(config: dict, full: dict) -> dict:
    """The decisions of `full` that differ from today's."""
    today = plant.current(_plant(config))
    return {k: v for k, v in full.items() if today[k] != v}


# ---- describe_model ----

def _describe(config, state, args):
    p, s = _plant(config), _study(config)
    lines = [{"id": ln["id"], "name": ln["name"], "product": ln["product"],
              "ideal_cycle_s": ln["ideal_cycle_s"], "variants": ln["variants"],
              "stations": [st["name"] for st in ln["stations"]]} for ln in p["lines"]]
    today = plant.current(p)
    decisions = [{**d, "current": today[d["name"]]} for d in p["decisions"]]
    limits = {"headcount_per_shift": p["headcount_per_shift"], "headcount_today": plant.headcount(today),
              "capex_budget_usd": p["capex"]["budget_usd"], "buffer_slot_usd": p["capex"]["buffer_slot_usd"],
              "safety": p["safety"],
              "after_simulation": [c for ln in p["lines"] for c in _outcome_rules(ln)]}
    return Done({"plant": p["company"], "week": p["week"], "lines": lines, "decisions": decisions,
                 "limits": limits, "kpis": {k: {"label": v[0], "unit": v[1]} for k, v in kpis.KPIS.items()},
                 "oee": "good units x ideal cycle time / all staffed time; planned maintenance, changeovers, "
                        "failures, waiting for a technician, blocking and starving all count as losses",
                 "objective_kpi": s["objective"], "budget": _budget(config, state)},
                reads=["model/plant", "study/budget"])


def _outcome_rules(line) -> list:
    out = []
    if "min_good_per_week" in line:
        out.append(f"{line['id']}.good_per_week >= {line['min_good_per_week']}")
    if "max_lead_time_h" in line:
        out.append(f"{line['id']}.lead_time_h <= {line['max_lead_time_h']}")
    return out


def _budget(config, state) -> dict:
    s = _study(config)
    used = len(state["simulations"])
    return {"simulations_used": used, "simulations_left": s["max_simulations"] - used,
            "max_simulations": s["max_simulations"], "replications_default": s["replications"]["default"],
            "replications_max": s["replications"]["max"]}


# ---- run_simulation ----

def _deny_config(config, full) -> None:
    broken = plant.violations(_plant(config), full)
    if not broken:
        return
    body = {"denied": True, "code": "CONFIG_REJECTED", "violations": broken,
            "message": "This config breaks the plant's hard limits; nothing was simulated and the "
                       "budget is unchanged."}
    raise Denied(body, "Denied: " + "; ".join(v["message"] for v in broken))


def _deny_budget(config, state) -> None:
    s = _study(config)
    if len(state["simulations"]) < s["max_simulations"]:
        return
    body = {"denied": True, "code": "SIMULATION_BUDGET_EXHAUSTED", "limit": s["max_simulations"],
            "message": f"All {s['max_simulations']} simulations of this study are used."}
    raise Denied(body, f"Denied: the study's {s['max_simulations']} simulations are used")


def _replications(config, asked) -> int:
    r = _study(config)["replications"]
    if asked is None:
        return r["default"]
    if not 1 <= asked <= r["max"]:
        raise Refusal(f"replications must be from 1 to {r['max']}")
    return asked


def _run(config, state, args):
    asked = {} if args["config"] is None else args["config"]
    bad = plant.malformed(_plant(config), asked)
    if bad:
        raise Refusal("; ".join(bad))
    full = plant.full(_plant(config), asked)
    _deny_config(config, full)
    _deny_budget(config, state)
    reps = _replications(config, args["replications"])
    out = simulate(config, full, list(range(reps)))
    n = len(state["simulations"]) + 1
    obj = objective(config, out["kpis"])
    record = {"simulation": n, "changes": changes(config, full), "config": full, "replications": reps,
              "objective": obj, "capex_usd": plant.capex(_plant(config), full), **out}
    state["simulations"].append(record)
    ci = f" ± {obj['ci95']:.1f}" if obj["ci95"] is not None else ""
    text = (f"Simulation {n}: {obj['label']} {obj['value']:.1f}{obj['unit']}{ci} "
            f"({reps} replications){'' if out['feasible'] else ', misses a limit'}")
    result = {**record, "budget": _budget(config, state)}
    return Done(result, writes=[f"simulation/{n}", "study/budget"], events=[event("simulation", text, {
        "simulation": n, "objective": obj, "changes": record["changes"], "feasible": out["feasible"]})])


# ---- get_history ----

def _brief(rec) -> dict:
    k = rec["kpis"]
    return {"simulation": rec["simulation"], "changes": rec["changes"], "replications": rec["replications"],
            "objective": rec["objective"], "feasible": rec["feasible"],
            "line1.good_per_week": k["line1.good_per_week"]["mean"],
            "line2.lead_time_h": k["line2.lead_time_h"]["mean"],
            "cost_per_week_usd": k["cost_per_week_usd"]["mean"], "capex_usd": rec["capex_usd"]}


def _history(config, state, args):
    sims = state["simulations"]
    feasible = [s for s in sims if s["feasible"]]
    best = max(feasible, key=lambda s: s["objective"]["value"], default=None)
    return Done({"simulations": [_brief(s) for s in sims],
                 "best_feasible": best["simulation"] if best else None,
                 "budget": _budget(config, state)},
                reads=[f"simulation/{s['simulation']}" for s in sims] + ["study/budget"])


# ---- submit_recommendation ----

def _submit(config, state, args):
    if state["recommendation"] is not None:
        raise Refusal("a recommendation was submitted already; it is final")
    n = args["simulation"]
    rec = next((s for s in state["simulations"] if s["simulation"] == n), None)
    if rec is None:
        raise Refusal(f"there is no simulation {n}; recommend a config you simulated (get_history)")
    if not args["summary"].strip():
        raise Refusal("summary must say what to change and why")
    s = _study(config)
    reps = list(range(s["verify_from"], s["verify_from"] + s["verify_replications"]))
    verified = simulate(config, rec["config"], reps)
    final = {"simulation": n, "changes": rec["changes"], "config": rec["config"], "summary": args["summary"],
             "violations": plant.violations(_plant(config), rec["config"]),
             "evidence": rec["objective"], "verified": {
                 "replications": len(reps), "objective": objective(config, verified["kpis"]), **verified}}
    state["recommendation"] = final
    v = final["verified"]["objective"]
    text = (f"Recommendation: simulation {n}; verified on {len(reps)} new replications: {v['label']} "
            f"{v['value']:.1f}{v['unit']} ± {v['ci95']:.1f}")
    return Done(final, writes=["recommendation/final"], events=[event("recommendation", text, {
        "simulation": n, "evidence": rec["objective"], "verified": v, "feasible": verified["feasible"]})])


TOOLS = {t.name: t for t in [
    Tool("describe_model", "The plant model: lines and stations, every decision you may change (today's "
         "value, bounds, unit), the hard limits (headcount, capex budget, safety minimums) and the limits "
         "judged after a simulation, the KPIs, and the simulation budget.", "read", (), _describe),
    Tool("run_simulation", "Simulate one week of the plant with a config: an object of decision names and "
         "values (left out: today's value), e.g. {\"line2.batch_size\": 200}. Returns each KPI's mean and "
         "95% confidence half-width over the replications (same random numbers for every config, so "
         "configs compare fairly), each station's time in each state, and whether the after-simulation "
         "limits hold. A config over a hard limit is denied and not counted. Each call uses one "
         "simulation of the budget.", "write",
         (Param("config", "object", None), Param("replications", "integer", None)), _run),
    Tool("get_history", "Every simulation so far: its changes, objective, feasibility and key KPIs, the best "
         "feasible one, and the budget left.", "read", (), _history),
    Tool("submit_recommendation", "Recommend the config of one of your simulations, with a short summary "
         "of what to change and why. It is final: the config is re-simulated on replications you have "
         "not seen, and that verified result is what counts.", "write",
         (Param("simulation", "integer"), Param("summary", "string")), _submit),
]}
