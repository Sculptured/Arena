"""The disruptions a task's setup may schedule in the carriers' world (arena_part.disruptions): a
lane that loses capacity, a lane that becomes unavailable, a transit delay on a lane, and the
supply chain director rejecting the next decision on an action."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import network

LANE = {"carrier": "string", "origin": "string", "destination": "string"}
ACTIONS = ("stock_reallocation", "cold_chain_shipment")
DEFAULT_NOTE = "Rejected: not now. Propose it again with the quantity and the orders it serves."


def _lane(config, do) -> dict:
    lane = network.find_lane(config, do["carrier"], do["origin"], do["destination"])
    if lane is None:
        raise Refusal(f"no lane of {do['carrier']} from {do['origin']} to {do['destination']}")
    return lane


def _change(config, state, do, **change) -> list:
    lane = _lane(config, do)
    state.setdefault("lane_changes", {}).setdefault(network.lane_key(lane), {}).update(change)
    return ["register/lanes"]


def _capacity(config, state, do) -> list:
    return _change(config, state, do, capacity=do["capacity"])


def _check_capacity(config, do) -> None:
    _lane(config, do)
    if do["capacity"] < 0:
        raise Refusal("capacity must be 0 or more")


def _unavailable(config, state, do) -> list:
    return _change(config, state, do, unavailable=True)


def _delay(config, state, do) -> list:
    """Later quotes on the lane take `extra_days` more, and so do the shipments booked on it."""
    lane = _lane(config, do)
    extra = network.changes(state, lane).get("extra_days", 0) + do["extra_days"]
    written = _change(config, state, do, extra_days=extra)
    for sid, s in state["shipments"].items():
        if (s["carrier"], s["origin"]) == (lane["carrier"], lane["origin"]):
            s["arrives"] = network.days_after_date(s["arrives"], do["extra_days"])
            s["delayed_days"] = s.get("delayed_days", 0) + do["extra_days"]
            written.append(f"shipment/{sid}")
    return written


def _check_delay(config, do) -> None:
    _lane(config, do)
    if do["extra_days"] < 1:
        raise Refusal("extra_days must be 1 or more")


def _reject_next(config, state, do) -> list:
    state.setdefault("reject_next", {})[do["action"]] = do.get("note") or DEFAULT_NOTE
    return []


def _check_action(config, do) -> None:
    if do["action"] not in ACTIONS:
        raise Refusal(f"action must be one of {', '.join(ACTIONS)}")


KINDS = {
    "lane_capacity": Kind({**LANE, "capacity": "integer"}, _capacity, _check_capacity),
    "lane_unavailable": Kind(LANE, _unavailable, _lane),
    "transit_delay": Kind({**LANE, "extra_days": "integer"}, _delay, _check_delay),
    "reject_next": Kind({"action": "string"}, _reject_next, _check_action),
}
