"""What changes the logistics state: quotes asked of the carriers, shipments booked on them and notices sent
to customers. Each function refuses (raises Refusal) before it changes anything."""
from arena_part.jsonstate import Done
from arena_part.params import Refusal

from . import network


def _next_id(state, kind: str, prefix: str) -> str:
    n = state["next"][kind]
    state["next"][kind] = n + 1
    return f"{prefix}-{n:04d}"


def quotes(config, state, args) -> Done:
    """A quote from every carrier with a lane from `origin` to the customer's region."""
    origin = network.origin_key(config, args["origin"])
    who = network.customer(config, args["customer"])
    if args["qty"] <= 0:
        raise Refusal("qty must be positive")
    found = network.lanes(config, origin, who["region"])
    if not found:
        raise Refusal(f"no carrier lane from {origin} to {who['name']} ({who['region']}) in the register")
    made = [_quote(config, state, lane, who["name"], args["qty"]) for lane in found]
    return Done({"quotes": made}, writes=[f"quote/{q['quote_id']}" for q in made])


def _quote(config, state, lane, customer, qty) -> dict:
    q = {"quote_id": _next_id(state, "quote", "Q"), "carrier": lane["carrier"], "service": lane["service"],
         "temperature": lane["temperature"], "origin": lane["origin"], "customer": customer, "qty": qty,
         "transit_days": lane["transit_days"], "ship_date": config["today"],
         "arrives": network.days_after(config, lane["transit_days"]), "price": lane["price"],
         "capacity_left": network.capacity_left(config, state, lane)}
    state["quotes"][q["quote_id"]] = q
    return q


def _bookable(config, state, args) -> dict:
    """The quote `args` books, once every condition holds."""
    quote = state["quotes"].get(args["quote_id"])
    if quote is None:
        raise Refusal(f"unknown quote {args['quote_id']}")
    if quote.get("shipment"):
        raise Refusal(f"quote {args['quote_id']} is booked already ({quote['shipment']})")
    if args["carrier"].strip().casefold() != quote["carrier"].casefold():
        raise Refusal(f"quote {args['quote_id']} is from {quote['carrier']}, not {args['carrier']}")
    if args["qty"] != quote["qty"]:
        raise Refusal(f"quote {args['quote_id']} is for {quote['qty']}, not {args['qty']}")
    lane = network.lane_of(config, quote["carrier"], quote["origin"])
    if network.capacity_left(config, state, lane) == 0:
        raise Refusal(f"{quote['carrier']} has no capacity left this week from {quote['origin']}")
    return quote


def book(config, state, args) -> Done:
    quote = _bookable(config, state, args)
    sid = _next_id(state, "shipment", "SHP")
    keep = ("quote_id", "carrier", "service", "temperature", "origin", "customer", "qty", "ship_date", "arrives")
    shipment = {"id": sid, **{k: quote[k] for k in keep}, "order": args["order"], "batch": args["batch"],
                "proposal_id": args["proposal_id"]}
    state["shipments"][sid] = shipment
    quote["shipment"] = sid
    return Done(shipment, writes=[f"shipment/{sid}", f"quote/{quote['quote_id']}"])


def notify(config, state, args) -> Done:
    who = network.customer(config, args["customer"])
    nid = _next_id(state, "notice", "N")
    notice = {"id": nid, "customer": who["name"], "order": args["order"],
              "new_date": network.iso_date(args["new_date"]), "message": args["message"]}
    state["notices"][nid] = notice
    return Done(notice, writes=[f"notice/{nid}"])
