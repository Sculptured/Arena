"""The disruption a task's setup may schedule (arena_part.disruptions): a supplier pushes an open purchase
order's arrival back."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import erp


def _po(state_or_config, po):
    for p in state_or_config["inbound"]:
        if p["po"] == po:
            return p
    raise Refusal(f"no open purchase order {po}")


def _check(config, do) -> None:
    _po(config, do["po"])
    erp.day(do["arrives"], "arrives")


def _delay(config, state, do) -> list:
    p = _po(state, do["po"])
    p["arrives"] = do["arrives"]
    state["suppliers"][p["supplier"]].update(status="delayed", note=do["note"])
    written = [f"item/{p['item']}", f"supplier/{p['supplier']}"]
    return written + ([f"order/{p['for_order']}"] if p.get("for_order") else [])


KINDS = {"supplier_delay": Kind({"po": "string", "arrives": "string", "note": "string"}, _delay, _check)}
