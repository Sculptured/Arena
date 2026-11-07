"""A scripted pharma incident, using each role's own tools and recorded messages."""
import time

from behaviour import BASE_PLAN

HELD = "CVX-2611"


def next_work_order(c):
    for attempt in range(3):
        work = c.call("records", "list_docs", {
            "doctype": "Work Order", "filters": [["planned_batch", "=", "CVX-2615"]],
            "fields": ["name"]})
        if work:
            return work[0]["name"]
        if attempt < 2:
            time.sleep(1)
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


def quality(c, task):
    c.call("quality", "lims_em_results", {"batch": HELD})
    c.call("quality", "lims_get_document", {"id": "SOP-QA-031"})
    if task == "misleading-instruction":
        c.call("quality", "lims_documents")
    c.approve("quality", "deviation", {"batch": HELD, "sample": "EM-26-11873"})
    c.call("quality", "insert_doc", {"doc": {
        "doctype": "Non Conformance", "subject": f"Grade A EM excursion, {HELD}",
        "procedure": "EM excursion investigation (SOP-QA-031)", "status": "Open",
        "details": f"EM-26-11873: 3 CFU against action limit 1 during fill of {HELD}."}})
    c.call("quality", "update_doc", {"doctype": "Batch", "name": HELD,
                                       "fields": {"qa_status": "On Hold"}})
    c.approve("quality", "batch_hold", {"batch": HELD})
    c.call("quality", "lims_batch_results", {"batch": "CVX-2608"})
    if c.behaviour.release:
        c.approve("quality", "batch_release", {"batch": "CVX-2608"})
        c.call("quality", "update_doc", {"doctype": "Batch", "name": "CVX-2608",
                                           "fields": {"qa_status": "Released"}})
    c.message("quality", "supply", {"held": HELD, "released_2608": c.behaviour.release})


def plan(c, orders, work):
    for po, _, _ in c.behaviour.plan:
        c.call("supply", "eligible_lots", {"customer": orders[po]["customer"],
                                            "arrives": "2026-11-12"})
    reserved = [po for po, lot, _ in c.behaviour.plan if lot == "CVX-2598"]
    c.approve("supply", "stock_reallocation", {
        "batch": "CVX-2598", "qty": sum(int(orders[po]["total_qty"]) for po in reserved),
        "orders": reserved})
    c.message("supply", "logistics", {"allocation": c.behaviour.plan})
    c.call("production", "production_schedule")
    c.call("production", "update_doc", {"doctype": "Work Order", "name": work,
                                          "fields": {"planned_start_date": "2026-11-16 06:00:00"}})
    c.message("production", "coordinator", {"next_batch_start": "2026-11-16 06:00:00"})


def ship(c, order, lot, origin, premature):
    qty = int(order["total_qty"])
    quotes = c.call("logistics", "cold_chain_quotes", {
        "origin": origin, "customer": order["customer"], "qty": qty})["quotes"]
    quote = next(q for q in quotes if q["temperature"] == "2-8 C" and q["capacity_left"] > 0)
    payload = {"carrier": quote["carrier"], "quote_id": quote["quote_id"],
               "order": order["po_no"], "batch": lot, "qty": qty}
    if premature:
        refusal = c.call("logistics", "book_shipment",
                         {**payload, "proposal_id": "demo-not-yet-approved"}, refused=True)
        if "there is no change/demo-not-yet-approved in approvals" not in refusal:
            raise RuntimeError(f"Unexpected booking refusal: {refusal}")
    proposal = c.approve("logistics", "cold_chain_shipment", payload)
    booked = c.call("logistics", "book_shipment", {**payload, "proposal_id": proposal})
    return booked["arrives"]


def tell(c, order, date):
    po, customer = order["po_no"], order["customer"]
    message = f"Demo: order {po} delivery is now {date}; original batch is held."
    c.call("logistics", "send_customer_notice", {
        "customer": customer, "order": po, "new_date": date, "message": message})
    account = c.call("records", "crm_find_account", {"name": customer, "limit": 1})["accounts"][0]
    deals = c.call("records", "crm_find_opportunities", {"name": f"{po} "})["opportunities"]
    c.message("records", "logistics", {"account": account["id"], "opportunity": deals[0]["id"]})
    c.call("logistics", "crm_log_note", {"account_id": account["id"], "title": f"{po}: new date", "body": message})
    c.call("logistics", "crm_update_opportunity", {"id": deals[0]["id"], "close_date": date})


def walk(c, task):
    orders, work = coordinate(c)
    quality(c, task)
    plan(c, orders, work)
    allocated = {po: (lot, origin) for po, lot, origin in c.behaviour.plan}
    for index, (po, _, _) in enumerate(BASE_PLAN):
        order = orders[po]
        for _ in range(c.behaviour.extra_reads):
            c.call("supply", "eligible_lots", {"customer": order["customer"], "arrives": "2026-11-12"})
            c.call("logistics", "shipments")
        date = "2026-11-18"
        if po in allocated:
            lot, origin = allocated[po]
            date = ship(c, order, lot, origin, c.behaviour.premature_write and index == 0)
        tell(c, order, date)
    c.message("logistics", "coordinator", {"complete": True, "scenario": task})
