"""Factory-01's ERP as data: the config (items, orders, suppliers, the purchase limit) and a state (stock,
reservations, the schedule, purchase orders, flags) per instance, and what each order and item looks like
now."""
from datetime import date, timedelta

from arena_part.params import Refusal

BASE = "base"


def check_config(config) -> None:
    for k in ("today", "po_limit_usd", "lines", "items", "orders", "suppliers"):
        if k not in config:
            raise Refusal(f"config needs {k}")


def new_state(config) -> dict:
    check_config(config)
    reserved: dict = {}
    for r in config.get("reservations", []):
        reserved.setdefault(r["order"], {})[r["item"]] = r["qty"]
    return {
        "scenario": BASE,
        "on_hand": {i["id"]: i["on_hand"] for i in config["items"]},
        "reserved": reserved,
        "starts": {o["id"]: o["start"] for o in config["orders"]},
        "suppliers": {s["id"]: {"status": s["status"], "note": None} for s in config["suppliers"]},
        "stock": {s["id"]: {k: v.get("stock") for k, v in s["offers"].items()} for s in config["suppliers"]},
        "inbound": [dict(p) for p in config.get("inbound", [])],
        "purchases": [],
        "flags": [],
        "log": [],
        "denials": [],
    }


def day(iso: str, field: str = "date") -> date:
    try:
        return date.fromisoformat(iso)
    except (TypeError, ValueError):
        raise Refusal(f"{field} must be a date YYYY-MM-DD, got {iso!r}") from None


def plus_days(iso: str, n: int) -> str:
    return (day(iso) + timedelta(days=n)).isoformat()


def by_id(rows, rid, what):
    for r in rows:
        if r["id"] == rid or r.get("name", "").lower() == str(rid).lower():
            return r
    raise Refusal(f"no {what} {rid!r} (known: {', '.join(r['id'] for r in rows)})")


def order(config, oid):
    return by_id(config["orders"], oid, "order")


def item(config, iid):
    return by_id(config["items"], iid, "item")


def supplier(config, sid):
    return by_id(config["suppliers"], sid, "supplier")


def reserved_total(state, iid) -> int:
    return sum(held.get(iid, 0) for held in state["reserved"].values())


def available(state, iid) -> int:
    return state["on_hand"][iid] - reserved_total(state, iid)


def covered(config, state, o, iid) -> int:
    """What order `o` has of item `iid` by its start: its reservation plus stock inbound for it by then."""
    start = day(state["starts"][o["id"]])
    inbound = sum(p["qty"] for p in state["inbound"]
                  if p["item"] == iid and p.get("for_order") == o["id"] and day(p["arrives"]) <= start)
    return state["reserved"].get(o["id"], {}).get(iid, 0) + inbound


def shortages(config, state, o) -> dict:
    out = {}
    for iid, per in o["bom"].items():
        short = per * o["qty"] - covered(config, state, o, iid)
        if short > 0:
            out[iid] = short
    return out


def flagged(state, oid) -> bool:
    return any(f["order_id"] == oid for f in state["flags"])


def order_doc(config, state, oid) -> dict:
    o = order(config, oid)
    short = shortages(config, state, o)
    start = state["starts"][o["id"]]
    late = day(start) > day(o["due"])
    status = "late" if late else "at risk" if short else "on track"
    return {"id": o["id"], "customer": o["customer"], "key_account": o["key_account"],
            "product": o["product"], "qty": o["qty"], "priority": o["priority"], "line": o["line"],
            "start": start, "due": o["due"], "status": status, "short": short,
            "reserved": state["reserved"].get(o["id"], {}), "flagged": flagged(state, o["id"])}


def item_doc(config, state, iid) -> dict:
    meta = item(config, iid)
    return {"id": meta["id"], "name": meta["name"], "on_hand": state["on_hand"][meta["id"]],
            "reserved": {oid: held[meta["id"]] for oid, held in state["reserved"].items()
                         if held.get(meta["id"])},
            "available": available(state, meta["id"]),
            "inbound": [p for p in state["inbound"] if p["item"] == meta["id"]]}


def supplier_doc(config, state, sid) -> dict:
    s = supplier(config, sid)
    offers = {}
    for iid, offer in s["offers"].items():
        o = {"unit_usd": offer["unit_usd"], "lead_days": offer["lead_days"],
             "arrives_if_ordered_today": plus_days(config["today"], offer["lead_days"])}
        if "expedite_unit_usd" in offer:
            o.update(expedite_surcharge_per_unit_usd=offer["expedite_unit_usd"],
                     expedite_lead_days=offer["expedite_lead_days"],
                     arrives_if_expedited_today=plus_days(config["today"], offer["expedite_lead_days"]))
        if state["stock"][s["id"]].get(iid) is not None:
            o["in_stock"] = state["stock"][s["id"]][iid]
        offers[iid] = o
    return {"id": s["id"], "name": s["name"], **state["suppliers"][s["id"]], "offers": offers,
            "open_pos": [p for p in state["inbound"] if p["supplier"] == s["id"]]}


def log(state, action: str, **fields) -> dict:
    entry = {"n": len(state["log"]) + 1, "action": action, **fields}
    state["log"].append(entry)
    return entry
