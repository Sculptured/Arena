"""The simulated supply chain director: decides every proposed change whose action the config does not
leave to another desk (`defer`), by fixed rules over this part's records and scenario, and gives a `decided`
event that a benchmark's link hands to the approvals system."""
from arena_part.jsonstate import Done, event

from . import network

REALLOCATION = "stock_reallocation"
SHIPMENT = "cold_chain_shipment"
SHIPMENT_FIELDS = ("carrier", "quote_id", "order", "batch", "qty")


def _approved(state, action, batch) -> list:
    return [r for r in state["reviews"].values()
            if r["action"] == action and r["approved"] and r["payload"].get("batch") == batch]


def _reallocation(config, state, payload):
    if not payload.get("batch") or not isinstance(payload.get("qty"), (int, float)):
        return False, "Rejected: name the batch and the quantity (qty) to reallocate."
    earlier = [r for r in state["reviews"].values() if r["action"] == REALLOCATION]
    first_note = network.overlay(config, state).get("reject_first_reallocation")
    if first_note and not earlier:
        return False, first_note
    return True, None


def _restricted_left(config, state, batch) -> float:
    """How much of a restricted batch approved reallocations leave for shipments not yet approved."""
    granted = sum(r["payload"]["qty"] for r in _approved(state, REALLOCATION, batch))
    shipped = sum(r["payload"]["qty"] for r in _approved(state, SHIPMENT, batch))
    return granted - shipped


def _lane_problem(config, state, payload):
    quote = state["quotes"].get(payload["quote_id"])
    if quote is None or quote["carrier"].casefold() != str(payload["carrier"]).casefold():
        return f"Rejected: no quote {payload['quote_id']} from {payload['carrier']}."
    lane = network.lane_of(config, quote["carrier"], quote["origin"])
    if not lane["qualified"]:
        return f"Rejected: {lane['carrier']}'s lane from {lane['origin']} is not qualified for 2-8 C."
    return None


def _shipment(config, state, payload):
    missing = [f for f in SHIPMENT_FIELDS if payload.get(f) in (None, "")]
    if missing:
        return False, f"Rejected: the payload needs {', '.join(missing)}."
    if not isinstance(payload["qty"], (int, float)):
        return False, "Rejected: qty must be a number."
    problem = _lane_problem(config, state, payload)
    if problem:
        return False, problem
    batch = payload["batch"]
    if batch in config["logistics"].get("restricted_batches", []) and _restricted_left(
            config, state, batch) < payload["qty"]:
        return False, f"Rejected: {batch} is reserved; it needs an approved {REALLOCATION} covering this quantity first."
    return True, None


def _decision(config, state, action, payload):
    if not isinstance(payload, dict):
        return False, "Rejected: the payload must be an object."
    if action == REALLOCATION:
        return _reallocation(config, state, payload)
    if action == SHIPMENT:
        return _shipment(config, state, payload)
    deferred = ", ".join(config.get("defer", []))
    return False, f"Rejected: this desk decides {REALLOCATION} and {SHIPMENT}; QA decides {deferred}."


def review(config, state, args) -> Done:
    """Decides the change `args` describes unless its action is another desk's."""
    change_id, action = args["id"], args["action"]
    key = f"review/{change_id}"
    if action in config.get("defer", []):
        return Done({"decided": False}, reads=[key])
    payload = args["payload"] if args["payload"] is not None else {}
    approved, note = _decision(config, state, action, payload)
    state["reviews"][change_id] = {"id": change_id, "action": action, "payload": payload,
                                   "approved": approved, "note": note, "by": "Supply chain director (simulated)"}
    verdict = "approved" if approved else "rejected"
    decided = event("decided", f"Supply chain director {verdict} {change_id} ({action})",
                    {"id": change_id, "approved": approved, "note": note})
    return Done({"decided": True, "approved": approved, "note": note}, writes=[key], events=[decided])
