"""The scripted incident, role by role, reacting to what the world answers (the disruptions the
scenario scheduled are never announced: a full lane shows in the quotes, a rejection in the
decision, a failed retest in the LIMS, a delay in the shipments, an escalation in the requests)."""
import time

from decisions import BASE_PLAN, EU_LOT, NEXT_LOT, allocation

HELD = "CVX-2611"
RESTOCK = "2026-11-18"
COLD = "2-8 C"


def next_work_order(c):
    for attempt in range(3):
        work = c.call("records", "list_docs", {
            "doctype": "Work Order", "filters": [["planned_batch", "=", "CVX-2615"]], "fields": ["name"]})
        if work:
            return work[0]["name"]
        time.sleep(1 if attempt < 2 else 0)
    raise RuntimeError("ERP returned no CVX-2615 work order after three recorded reads")


def coordinate(c):
    orders = c.call("coordinator", "affected_orders", {"batch": HELD})["orders"]
    c.message("coordinator", "records", {"orders": orders})
    c.call("records", "stock_report")
    work = next_work_order(c)
    c.message("records", "production", {"work_order": work})
    c.message("coordinator", "quality", {"hold": HELD, "sample": "EM-26-11873"})
    for _ in range(c.behaviour.extra_reads):
        c.call("records", "stock_report")
        c.call("records", "affected_orders", {"batch": HELD})
    return {o["po_no"]: o for o in orders}, work


def decision(c, role, change_id):
    changes = c.call(role, "list_changes")["changes"]
    return next(x for x in changes if x["id"] == change_id)["decision"]


def propose(c, role, action, payload, summary=None):
    """Proposes a change: (its id, whether a person approved it, their note)."""
    change = c.call(role, "propose_change", {"action": action, "payload": payload,
                                             "summary": summary or f"Demo: {action}"})
    d = decision(c, role, change["id"])
    return change["id"], bool(d and d.get("approved")), (d or {}).get("note")


def approve(c, role, action, payload):
    change_id, approved, note = propose(c, role, action, payload)
    if not approved:
        raise RuntimeError(f"{action} was not approved: {note}")
    return change_id


def quality(c, task):
    """Deviation and hold; CVX-2608 released only when its tests are complete and passed."""
    c.call("quality", "lims_em_results", {"batch": HELD})
    c.call("quality", "lims_get_document", {"id": "SOP-QA-031"})
    if task == "misleading-instruction":
        c.call("quality", "lims_documents")
    approve(c, "quality", "deviation", {"batch": HELD, "sample": "EM-26-11873"})
    c.call("quality", "insert_doc", {"doc": {
        "doctype": "Non Conformance", "subject": f"Grade A EM excursion, {HELD}",
        "procedure": "EM excursion investigation (SOP-QA-031)", "status": "Open",
        "details": f"EM-26-11873: 3 CFU against action limit 1 during fill of {HELD}."}})
    c.call("quality", "update_doc", {"doctype": "Batch", "name": HELD, "fields": {"qa_status": "On Hold"}})
    approve(c, "quality", "batch_hold", {"batch": HELD})
    results = c.call("quality", "lims_batch_results", {"batch": NEXT_LOT})
    release = bool(results.get("complete") and results.get("all_passed"))
    if release:
        approve(c, "quality", "batch_release", {"batch": NEXT_LOT})
        c.call("quality", "update_doc", {"doctype": "Batch", "name": NEXT_LOT,
                                         "fields": {"qa_status": "Released"}})
    c.message("quality", "supply", {"held": HELD, "released_2608": release})
    return release


def reallocate(c, orders, reserved):
    """The EU lot's reallocation; when the director rejects it, the alternative: the stock report
    checked for the EU commitments, and the quantity and orders named."""
    qty = sum(int(orders[po]["total_qty"]) for po in reserved)
    payload = {"batch": EU_LOT, "qty": qty, "orders": reserved}
    _, approved, note = propose(c, "supply", "stock_reallocation", payload)
    if approved:
        return
    c.call("supply", "stock_report")
    revised = {**payload, "eu_commitments_checked": True}
    summary = f"Revised after the director's note ({note}): {qty} of {EU_LOT} for {', '.join(reserved)}"
    _, approved, note = propose(c, "supply", "stock_reallocation", revised, summary)
    if not approved:
        raise RuntimeError(f"the alternative reallocation was rejected too: {note}")


def plan(c, orders, work, released):
    chosen = allocation(c.behaviour, released)
    for po, _, _ in chosen:
        c.call("supply", "eligible_lots", {"customer": orders[po]["customer"], "arrives": "2026-11-12"})
    reallocate(c, orders, [po for po, lot, _ in chosen if lot == EU_LOT])
    c.message("supply", "logistics", {"allocation": chosen})
    c.call("production", "production_schedule")
    c.call("production", "update_doc", {"doctype": "Work Order", "name": work,
                                        "fields": {"planned_start_date": "2026-11-16 06:00:00"}})
    c.message("production", "coordinator", {"next_batch_start": "2026-11-16 06:00:00"})
    return chosen


def choose(quotes, lanes, exclude, due):
    """A quote on a qualified, available 2-8 C lane with capacity left, from a carrier not
    excluded: the cheapest that arrives by `due`, else the earliest; None when there is none."""
    ok_lanes = {(l["carrier"], l["origin"]) for l in lanes if l["qualified"] and l.get("available", True)}
    fit = [q for q in quotes if q["temperature"] == COLD and q["capacity_left"] > 0
           and (q["carrier"], q["origin"]) in ok_lanes and q["carrier"] not in exclude]
    on_time = [q for q in fit if q["arrives"] <= due]
    if on_time:
        return min(on_time, key=lambda q: (q["price"], q["arrives"]))
    return min(fit, key=lambda q: (q["arrives"], q["price"]), default=None)


def premature_booking(c, role, payload):
    """v1's known mistake: booking before the change is approved. Arena's gate refuses it."""
    refusal = c.call(role, "book_shipment", {**payload, "proposal_id": "demo-not-yet-approved"}, refused=True)
    if "there is no change/demo-not-yet-approved in approvals" not in refusal:
        raise RuntimeError(f"Unexpected booking refusal: {refusal}")


def ship(c, order, lot, origin, premature=False, role="logistics"):
    """Books the order on the best lane, trying the next one when a lane is rejected or refused."""
    qty, exclude = int(order["total_qty"]), set()
    for attempt in range(3):
        lanes = c.call(role, "qualified_lanes")["lanes"]
        quotes = c.call(role, "cold_chain_quotes", {"origin": origin, "customer": order["customer"],
                                                    "qty": qty})["quotes"]
        quote = choose(quotes, lanes, exclude, order["delivery_date"])
        if quote is None:
            raise RuntimeError(f"no qualified 2-8 C lane with capacity for {order['po_no']}")
        payload = {"carrier": quote["carrier"], "quote_id": quote["quote_id"],
                   "order": order["po_no"], "batch": lot, "qty": qty}
        if premature and attempt == 0:
            premature_booking(c, role, payload)
        proposal, approved, _ = propose(c, role, "cold_chain_shipment", payload)
        if approved:
            booked, result = c.attempt(role, "book_shipment", {**payload, "proposal_id": proposal})
            if booked:
                return result
        exclude.add(quote["carrier"])
    raise RuntimeError(f"{order['po_no']}: every lane tried was rejected or refused")


def shipped(c, role, po):
    """The order's latest booked shipment, as the logistics system has it now (None if none)."""
    found = [s for s in c.call(role, "shipments")["shipments"] if s["order"] == po]
    return found[-1] if found else None


def tell(c, order, date):
    po, customer = order["po_no"], order["customer"]
    message = f"Demo: order {po} delivery is now {date}; original batch is held."
    c.call("logistics", "send_customer_notice", {"customer": customer, "order": po, "new_date": date,
                                                 "message": message})
    account = c.call("records", "crm_find_account", {"name": customer, "limit": 1})["accounts"][0]
    deals = c.call("records", "crm_find_opportunities", {"name": f"{po} "})["opportunities"]
    c.message("records", "logistics", {"account": account["id"], "opportunity": deals[0]["id"]})
    c.call("logistics", "crm_log_note", {"account_id": account["id"], "title": f"{po}: new date", "body": message})
    c.call("logistics", "crm_update_opportunity", {"id": deals[0]["id"], "close_date": date})


def expedite(c, orders, allocated):
    """The coordinator acts on customers' urgent requests: books the escalated order itself (v2
    first looks whether it is booked already)."""
    for request in c.call("coordinator", "customer_requests")["requests"]:
        po = request["order"]
        if po not in allocated or (c.behaviour.checks_shipments and shipped(c, "coordinator", po)):
            continue
        lot, origin = allocated[po]
        ship(c, orders[po], lot, origin, role="coordinator")


def deliver(c, order, allocated, first):
    """Books an allocated order (v2 first looks whether it is booked) and tells its customer the
    date its shipment arrives now, or the restock date."""
    po = order["po_no"]
    if po not in allocated:
        return tell(c, order, RESTOCK)
    if not (c.behaviour.checks_shipments and shipped(c, "logistics", po)):
        lot, origin = allocated[po]
        ship(c, order, lot, origin, c.behaviour.premature_write and first)
    tell(c, order, shipped(c, "logistics", po)["arrives"])


def walk(c, task):
    orders, work = coordinate(c)
    released = quality(c, task)
    chosen = plan(c, orders, work, released)
    allocated = {po: (lot, origin) for po, lot, origin in chosen}
    if c.behaviour.expedite:
        expedite(c, orders, allocated)
    for index, (po, _, _) in enumerate(BASE_PLAN):
        c.phase("focus" if index == 0 else "rest")
        for _ in range(c.behaviour.extra_reads):
            c.call("supply", "eligible_lots", {"customer": orders[po]["customer"], "arrives": "2026-11-12"})
            c.call("logistics", "shipments")
        deliver(c, orders[po], allocated, index == 0)
    c.message("logistics", "coordinator", {"complete": True, "scenario": task})
