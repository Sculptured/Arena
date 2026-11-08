"""The disruption a task's setup may schedule at the utility (arena_part.disruptions): a demand-response event,
a window of peak prices in which the site is asked to hold its load at or below a target. Its notice arrives when
it fires; the window opens `lead_min` minutes later and lasts `minutes`."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import meter

PROGRAM = "Peak-demand response"


def _check(config, do) -> None:
    if do["target_kw"] < 0:
        raise Refusal("target_kw must be 0 or more")
    if do["lead_min"] < 0 or do["minutes"] < 1:
        raise Refusal("lead_min must be 0 or more and minutes 1 or more")


def _event(config, state, do) -> list:
    now = state["clock_ms"]
    start = now + do["lead_min"] * meter.MINUTE_MS
    ev = {"id": do["event"], "program": do.get("program") or PROGRAM, "target_w": meter.watts(do["target_kw"]),
          "notice_ms": now, "start_ms": start, "end_ms": start + do["minutes"] * meter.MINUTE_MS,
          "ms_in": 0, "wms_in": 0, "ms_over": 0, "max_w": 0, "response": None}
    ev["status"] = meter.status_at(ev, now)
    state["events"][ev["id"]] = ev
    return [f"dr/{ev['id']}"]


KINDS = {
    "demand_response": Kind({"event": "string", "target_kw": "number", "lead_min": "integer", "minutes": "integer"},
                            _event, _check),
}
