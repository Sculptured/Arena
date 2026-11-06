"""The arena-part/1 integration: a config (the benchmark's network data) and a state (scenario, quotes,
shipments, notices, the director's reviews) per instance, saved as one JSON file."""
from arena_part.jsonstate import JsonStatePart

from . import network
from .tools import TOOLS

VERSION = "0.1.0"
KINDS = {"quote": "quotes", "shipment": "shipments", "notice": "notices", "review": "reviews"}


class PharmaLogistics(JsonStatePart):
    NAME = "pharma-logistics"
    TOOLS = TOOLS

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "logistics", "product": "Simulated cold-chain carriers and customer desk",
                           "simulated": True}}

    def new_state(self, config):
        network.check_config(config)
        return {"scenario": network.BASE, "quotes": {}, "shipments": {}, "notices": {}, "reviews": {},
                "next": {"quote": 1, "shipment": 1, "notice": 1}}

    def docs(self, config, state):
        out = {"scenario/current": {"name": state["scenario"]}}
        for kind, field in KINDS.items():
            out.update({f"{kind}/{k}": v for k, v in state[field].items()})
        return out
