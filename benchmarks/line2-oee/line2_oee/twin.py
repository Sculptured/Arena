"""The study's digital twin (docs/protocol.md, "The world's view"): both lines' stations as the latest
simulation saw them, the study's best feasible candidate, the recommendation and any denial."""


def _item(iid, label, status, abnormal=False, **extra) -> dict:
    return {"id": iid, "label": label, "status": status, "abnormal": bool(abnormal), **extra}


def _sims(sim: dict) -> list:
    out = [d for k, d in sim.items() if k.startswith("simulation/") and d]
    return sorted(out, key=lambda d: d["simulation"])


def _line(config: dict, line: dict, latest: dict | None) -> dict:
    rows = (latest or {}).get("stations", {}).get(line["id"])
    items = []
    for i, st in enumerate(line["stations"]):
        iid = f"{line['id']}:{st['id']}"
        if not rows:
            items.append(_item(iid, st["name"], "not simulated yet"))
            continue
        r = rows[i]
        losses = ", ".join(f"{k} {r[k]:.0f}%" for k in ("changeover", "down", "pm", "blocked", "starved")
                           if r[k] >= 1)
        items.append(_item(iid, st["name"], f"{r['working']:.0f}% working", r["down"] >= 10,
                           detail=losses or "no losses"))
    return {"id": line["id"], "label": f"{line['name']} · {line['product']}", "kind": "plant", "items": items}


def _study(sims: list, rec: dict | None, denials: list) -> dict:
    items = []
    feasible = [s for s in sims if s["feasible"]]
    if feasible:
        best = max(feasible, key=lambda s: s["objective"]["value"])
        o = best["objective"]
        items.append(_item("best", f"Best · simulation {best['simulation']}", f"{o['value']:.1f}{o['unit']}",
                           detail=", ".join(best["changes"]) or "today's config"))
    if rec and "verified" in rec:
        v = rec["verified"]
        o = v["objective"]
        items.append(_item("recommendation", f"Recommended · simulation {rec['simulation']}",
                           f"verified {o['value']:.1f}{o['unit']}", not v["feasible"], detail=rec["summary"][:200]))
    items += [_item(f"denial:{d['n']}", "Denied", d["code"].replace("_", " ").lower(), True,
                    detail=d["message"]) for d in denials]
    return {"id": "study", "label": "Study", "kind": "approvals", "items": items}


def _pct(v) -> str:
    return "–" if v is None else f"{v:.1f}%"


def _kpis(config: dict, sims: list, rec: dict | None, budget: dict) -> list:
    feasible = [s["objective"]["value"] for s in sims if s["feasible"]]
    latest = sims[-1]["objective"] if sims else None
    verified = rec["verified"]["objective"]["value"] if rec and "verified" in rec else None
    return [
        {"id": "latest", "label": "Line 2 OEE, latest", "value": _pct(latest["value"] if latest else None), "abnormal": False},
        {"id": "best", "label": "Best feasible", "value": _pct(max(feasible, default=None)), "abnormal": False},
        {"id": "verified", "label": "Recommended, verified", "value": _pct(verified), "abnormal": False},
        {"id": "simulations", "label": "Simulations", "value": f"{budget.get('used', 0)} / "
         f"{budget.get('limit', config['study']['max_simulations'])}", "abnormal": False},
    ]


def view(config: dict, docs: dict) -> dict:
    sim = docs.get("sim", {})
    sims = _sims(sim)
    rec = sim.get("recommendation/final")
    denials = sim.get("_audit/denials", {}).get("denials", [])
    latest = sims[-1] if sims else None  # the stations as the latest simulation saw them
    areas = [_line(config, ln, latest) for ln in config["lines"]] + [_study(sims, rec, denials)]
    return {"title": f"{config['company']} · Line 2 OEE study", "areas": areas, "flows": [], "kpis": _kpis(config, sims, rec, sim.get("study/budget", {}))}
