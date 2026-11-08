"""The disruption a task's setup may schedule on the roster (arena_part.disruptions): operators call out, so they
are off shift, and the stations they staffed are not staffed any more."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import roster


def _check(config, do) -> None:
    names = do["operators"]
    if not names or not all(isinstance(n, str) for n in names):
        raise Refusal("operators must be a list of operator ids or names")
    for n in names:
        roster.operator(config, roster.new_state(config), n)


def _callout(config, state, do) -> list:
    written = []
    for named in do["operators"]:
        oid = roster.operator(config, state, named)["id"]
        state["operators"][oid].update(on_shift=False, called_out=True)
        written.append(f"operator/{oid}")
        station = state["operators"][oid]["station"]
        if station:
            written.append(f"station/{station}")
    return written


KINDS = {"callout": Kind({"operators": "array"}, _callout, _check)}
