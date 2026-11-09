"""Factory-01's digital twin (docs/protocol.md, "The world's view"): the lines, the orders, the stock, the
suppliers and what went to a human, drawn from the ERP's docs after every writing step."""
from .checks import evaluate


def _item(iid, label, status, abnormal, **extra) -> dict:
    return {"id": iid, "label": label, "status": status, "abnormal": bool(abnormal), **extra}


def _orders(docs: list) -> dict:
    items = [_item(f"order:{o['id']}", f"{o['id']} · {o['customer']}",
                   o["status"] + (", flagged" if o["flagged"] else ""), o["status"] != "on track",
                   detail=f"{o['qty']} {o['product']} · {o['priority']} priority"
                   + (f" · short {', '.join(f'{n} {i}' for i, n in o['short'].items())}" if o["short"] else ""),
                   qty=o["qty"], unit="units", due=o["due"]) for o in docs]
    return {"id": "orders", "label": "Orders", "kind": "customers", "items": items}


def _lines(lines: list, orders: list) -> dict:
    items = []
    for ln in lines:
        mine = sorted((o for o in orders if o["line"] == ln["id"]), key=lambda o: o["start"])
        bad = [o["id"] for o in mine if o["status"] != "on track"]
        status = f"{', '.join(bad)} at risk" if bad else "on plan"
        items.append(_item(f"line:{ln['id']}", f"{ln['name']} · {ln['makes']}", status, bad,
                           detail=" → ".join(f"{o['id']} {o['start'][5:]}" for o in mine)))
    return {"id": "lines", "label": "Lines", "kind": "plant", "items": items}


def _stock(items: list) -> dict:
    rows = [_item(f"item:{i['id']}", i["name"], f"{i['available']} available of {i['on_hand']}",
                  i["available"] < 0, qty=i["available"], unit="available",
                  detail=", ".join(f"{n} for {o}" for o, n in i["reserved"].items()) or "none reserved")
            for i in items]
    return {"id": "stock", "label": "Inventory", "kind": "warehouse", "items": rows}


def _suppliers(rows: list) -> dict:
    items = [_item(f"supplier:{s['id']}", s["name"], s["status"], s["status"] != "on time",
                   detail=s.get("note") or f"{len(s['open_pos'])} open PO(s)") for s in rows]
    return {"id": "suppliers", "label": "Suppliers", "kind": "carriers", "items": items}


def _human(erp: dict) -> dict:
    flags = erp.get("flags/all", {}).get("flags", [])
    denials = erp.get("_audit/denials", {}).get("denials", [])
    items = [_item(f"flag:{f['n']}", f"Flag · {f['order_id']}", "with a human", False,
                   detail=f["reason"], link=f"order:{f['order_id']}") for f in flags]
    items += [_item(f"denial:{d['n']}", f"Denied · USD {d['requested_usd']:,.0f}",
                    f"over the USD {d['limit_usd']:g} limit", True, detail=d["message"]) for d in denials]
    return {"id": "human", "label": "Limits and flags", "kind": "approvals", "items": items}


def _flows(items: list, suppliers: list) -> list:
    delayed = {s["id"] for s in suppliers if s["status"] == "delayed"}
    out = []
    for i in items:
        for po in i["inbound"]:
            late = po["supplier"] in delayed
            out.append({"id": f"po:{po['po']}", "from": f"supplier:{po['supplier']}", "to": f"item:{i['id']}",
                        "label": f"{po['po']} · {po['qty']} {i['id']} · arrives {po['arrives'][5:]}",
                        "status": "delayed" if late else "ordered", "abnormal": late})
    return out


def _kpis(config: dict, erp: dict) -> list:
    pos = erp.get("purchase/orders", {}).get("orders", [])
    checks = {c["name"]: c for c in evaluate(config, {"erp": erp})}
    split = not checks["no_split_purchases"]["passed"]
    return [
        {"id": "po_limit", "label": "PO limit", "value": f"USD {config['po_limit_usd']:g} / order",
         "abnormal": False},
        {"id": "spend", "label": "Spend", "value": f"USD {sum(p['total_usd'] for p in pos):,.0f} in {len(pos)} PO(s)",
         "abnormal": split},
        {"id": "denials", "label": "Denials", "value": str(len(erp.get("_audit/denials", {}).get("denials", []))),
         "abnormal": False},
        {"id": "flags", "label": "Flags", "value": str(len(erp.get("flags/all", {}).get("flags", []))),
         "abnormal": False},
        {"id": "violations", "label": "Safety violations",
         "value": str(sum(1 for c in checks.values() if c["safety"] and not c["passed"])),
         "abnormal": any(c["safety"] and not c["passed"] for c in checks.values())},
    ]


def view(config: dict, docs: dict) -> dict:
    erp = docs.get("erp", {})
    orders = [erp[f"order/{o['id']}"] for o in config["orders"]]
    items = [erp[f"item/{i['id']}"] for i in config["items"]]
    suppliers = [erp[f"supplier/{s['id']}"] for s in config["suppliers"]]
    return {"title": f"Factory-01 · {config['company']}", "clock": config["today"],
            "areas": [_lines(config["lines"], orders), _orders(orders), _stock(items),
                      _suppliers(suppliers), _human(erp)],
            "flows": _flows(items, suppliers), "kpis": _kpis(config, erp)}
