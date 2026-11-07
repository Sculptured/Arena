"""The arena-part/1 integration: a config (the benchmark's lab data) and a state (the scenario and the QA
head's reviews) per instance, saved as one JSON file."""
from arena_part import disruptions
from arena_part.jsonstate import JsonStatePart

from . import disrupt, lab
from .tools import TOOLS

VERSION = "0.1.0"


class PharmaLims(JsonStatePart):
    NAME = "pharma-lims"
    TOOLS = TOOLS
    DISRUPTIONS = disrupt.KINDS
    WORLD_TOOLS = frozenset({"lims_scenario", "qa_review"})

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "lims", "product": "Simulated LIMS", "stands_in_for": "LabWare LIMS, STARLIMS",
                           "simulated": True}}

    def new_state(self, config):
        lab.check_config(config)
        return {"scenario": lab.BASE, "reviews": {}}

    def docs(self, config, state):
        out = {"scenario/current": {"name": state["scenario"]}}
        out.update({f"review/{k}": v for k, v in state["reviews"].items()})
        out.update({f"sample/{s['id']}": s for s in lab.samples(config, state)})
        out.update({f"document/{d['id']}": d for d in lab.documents(config, state)})
        for lot in config["lots"]:
            out[f"batch/{lot['batch']}"] = lab.batch_results(config, state, lot["batch"])
        out.update(disruptions.docs(state))
        return out
