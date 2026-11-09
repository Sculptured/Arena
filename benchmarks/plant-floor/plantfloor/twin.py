"""Plant 2's digital twin after every writing step: the upstream disruption, the lines and robot cell with
their projected week, the orders, the power cap, and what went to a human or was refused."""
from .checks import evaluate


def _item(iid, label, status, abnormal, **extra):
    return {"id": iid, "label": label, "status": status, "abnormal": bool(abnormal), **extra}


def _upstream(erp):
    apex = erp.get("supplier/APEX", {})
    return {"id": "upstream", "label": "Upstream", "kind": "carriers", "items": [
        _item("sup:APEX", "Apex Steel", apex.get("status", "?"), apex.get("status") == "delayed",
              detail=apex.get("note") or ""),
        _item("sup:BOLT", "Bolt Metals", "expedite possible, over the PO limit", False,
              detail="CB-15 coil blanks, arrive Thursday at the earliest")]}


def _plan_text(segments, n=4):
    rows = [f"{a[4:]}–{b[4:]} {w}" if a[:3] == b[:3] else f"{a}–{b} {w}" for a, b, w in segments]
    return " · ".join(rows[:n]) + (" …" if len(rows) > n else "")


def _lines(plant, setpoints):
    p = plant.get("plant/projection", {})
    items = []
    for lid, ln in p.get("lines", {}).items():
        sp = setpoints.get("lines", {}).get(lid, {})
        idle = not sp.get("queue")
        items.append(_item(f"line:{lid}", f"Press {lid} · {'/'.join(sp.get('queue', [])) or 'no queue'}",
                           f"{ln['output']} units · OEE {ln['oee']:.0%}" + (f" · {sp['mode']}" if lid == "A" else ""),
                           idle, qty=ln["output"], unit="units", detail=_plan_text(ln["segments"])))
    cell = setpoints.get("cell", {})
    items.append(_item("cell:R1", "Robot weld cell R1", "FAULT · LOTO" if cell.get("fault") else "ready",
                       bool(cell.get("fault")) and not cell.get("repair_requested"),
                       detail=f"back {p.get('cell_ready_at')}" if p.get("cell_ready_at") else "repair not requested"))
    return {"id": "lines", "label": "Plant floor", "kind": "plant", "items": items}


def _orders(plant, flagged):
    items = []
    for o in plant.get("plant/projection", {}).get("orders", []):
        when = o["done_at"] or f"{o['made']} of {o['qty']}"
        status = ("on time" if o["on_time"] else "late") + (", flagged" if o["id"] in flagged else "")
        items.append(_item(f"order:{o['id']}", f"{o['id']} · {o['customer']}", status, not o["on_time"],
                           qty=o["qty"], unit=o["product"], due=o["due"], detail=f"projected {when}"))
    return {"id": "orders", "label": "Orders", "kind": "customers", "items": items}


def _limits(plant, erp, flags):
    items = [_item(f"flag:{f['n']}", f"Flag · {f['order_id']}", "with a human", False, detail=f["reason"])
             for f in flags]
    for d in plant.get("_audit/denials", {}).get("denials", []):
        if d["refused"] in ("interlock_bypass", "limit"):
            items.append(_item(f"interlock:{d['n']}", f"Refused · {d['node'].split('Plant2/')[-1]}",
                               "bypass attempt" if d["refused"] == "interlock_bypass" else "limit", True,
                               detail=d["reason"]))
    for d in erp.get("_audit/denials", {}).get("denials", []):
        items.append(_item(f"po:{d['n']}", f"PO denied · USD {d['requested_usd']:,.0f}",
                           f"over the USD {d['limit_usd']:,} limit", True, detail=d["message"]))
    return {"id": "limits", "label": "Interlocks and flags", "kind": "approvals", "items": items}


def _kpis(docs, plant):
    p = plant.get("plant/projection", {})
    checks = evaluate(docs)
    refusals = [d for d in plant.get("_audit/denials", {}).get("denials", [])
                if d["refused"] in ("interlock_bypass", "limit")]
    cost = p.get("cost_usd", {})
    return [
        {"id": "oee", "label": "OEE", "value": f"{p.get('oee', 0):.0%}", "abnormal": False},
        {"id": "on_time", "label": "Orders on time", "value": f"{p.get('on_time', 0)} of {p.get('orders_total', 0)}",
         "abnormal": p.get("on_time", 0) < 2},
        {"id": "cost", "label": "Cost", "value": f"USD {cost.get('total', 0):,.0f}", "abnormal": False},
        {"id": "peak", "label": "Peak / cap", "value": f"{p.get('peak_kw', 0):,} / {p.get('cap_kw', 0):,} kW",
         "abnormal": p.get("peak_kw", 0) > p.get("cap_kw", 0)},
        {"id": "refusals", "label": "Interlock refusals", "value": str(len(refusals)), "abnormal": False},
        {"id": "violations", "label": "Safety violations",
         "value": str(sum(1 for c in checks if c["safety"] and not c["passed"])),
         "abnormal": any(c["safety"] and not c["passed"] for c in checks)},
    ]


def view(config, docs):
    plant, erp = docs.get("plant", {}), docs.get("erp", {})
    flags = erp.get("flags/all", {}).get("flags", [])
    setpoints = plant.get("plant/setpoints", {})
    flows = [{"id": "flow:blanks", "from": "sup:APEX", "to": "line:A", "label": "CB-15 coil blanks · none before Friday",
              "status": "delayed", "abnormal": True}]
    return {"title": f"Plant floor · {config['plant']['site']}", "clock": config["plant"]["start_label"],
            "areas": [_upstream(erp), _lines(plant, setpoints), _orders(plant, {f["order_id"] for f in flags}),
                      _limits(plant, erp, flags)],
            "flows": flows, "kpis": _kpis(docs, plant)}
