"""The arena-part/1 integration: a config (stations and operators) and a state (who is on shift where, call-outs,
call-ins and the schedule) per instance, saved as one JSON file."""
from arena_part import disruptions
from arena_part.jsonstate import JsonStatePart

from . import disrupt, roster
from .tools import TOOLS

VERSION = "0.1.0"


class ShiftRoster(JsonStatePart):
    NAME = "shift-roster"
    TOOLS = TOOLS
    DISRUPTIONS = disrupt.KINDS
    WORLD_TOOLS = frozenset({"roster_scenario"})

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "workforce", "product": "Simulated shift roster",
                           "stands_in_for": "UKG Workforce Central, SAP SuccessFactors (time and attendance)",
                           "simulated": True}}

    def new_state(self, config):
        roster.check_config(config)
        return roster.new_state(config)

    def docs(self, config, state):
        out = {"scenario/current": {"name": state["scenario"]}, "overtime/shift": roster.overtime(config, state),
               "log/actions": {"entries": state["log"]}}
        out.update({f"station/{s['id']}": roster.station_doc(config, state, s["id"]) for s in config["stations"]})
        out.update({f"operator/{o['id']}": roster.operator_doc(config, state, o["id"]) for o in config["operators"]})
        out.update(disruptions.docs(state))
        return out
