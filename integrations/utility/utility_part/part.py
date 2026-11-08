"""The arena-part/1 integration: a config (the site, the tariff, the load at the start) and a state (the meter,
the demand-response events and the schedule) per instance, saved as one JSON file. On the arena clock: the meter
integrates the load as the clock runs, and a window's start and end are world events."""
from arena_part import disruptions
from arena_part.jsonstate import JsonStatePart

from . import disrupt, meter
from .tools import BASE, TOOLS

VERSION = "0.1.0"


class Utility(JsonStatePart):
    NAME = "utility"
    TOOLS = TOOLS
    DISRUPTIONS = disrupt.KINDS
    WORLD_TOOLS = frozenset({"meter_load", "utility_scenario"})

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "utility", "product": "Simulated electric utility and site meter",
                           "stands_in_for": "a utility's demand-response program and a revenue meter",
                           "simulated": True}}

    def describe(self):
        return {**super().describe(), "uses_clock": True}

    def new_state(self, config):
        meter.check_config(config)
        return {"scenario": BASE, "clock_ms": 0, "meter": meter.new_meter(config), "events": {}, "queued": []}

    def start(self, instance, config, seed, clock_ms, from_=None):
        super().start(instance, config, seed, clock_ms, from_)
        state = self._instance(instance).state
        if from_ is None:
            state["clock_ms"] = clock_ms

    def _run_to(self, inst, clock_ms) -> None:
        """The meter up to `clock_ms`; the window events wait for the next advance."""
        inst.state["queued"] += meter.run_to(inst.config, inst.state, clock_ms)

    def advance(self, instance, clock_ms):
        inst = self._instance(instance)
        before = inst.state["clock_ms"]
        self._run_to(inst, clock_ms)
        events, inst.state["queued"] = inst.state["queued"], []
        return {"changed": bool(events) or inst.state["clock_ms"] != before, "events": events}

    def call_tool(self, instance, name, args, clock_ms):
        self._run_to(self._instance(instance), clock_ms)
        return super().call_tool(instance, name, args, clock_ms)

    def docs(self, config, state):
        out = {"scenario/current": {"name": state["scenario"]}, "meter/site": meter.meter_doc(config, state)}
        out.update({f"dr/{k}": meter.event_doc(config, ev) for k, ev in state["events"].items()})
        out.update(disruptions.docs(state))
        return out
