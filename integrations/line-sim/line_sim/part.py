"""The arena-part/1 integration: a simulation study of a plant. Each instance is the plant's config
(with its study settings) and one JSON state: every simulation run, the recommendation, and the
denials. A config over a hard limit, or a simulation past the budget, is refused with a structured
denial, recorded, and reported to the world as a `denial` event."""
from arena_part import errors
from arena_part.jsonstate import JsonStatePart, event

from .backend import backend_for
from .tools import TOOLS, Denied

VERSION = "0.1.0"
DENIALS = "_audit/denials"
STUDY = ("max_simulations", "replications", "verify_replications", "verify_from", "objective")


class LineSim(JsonStatePart):
    NAME = "line-sim"
    TOOLS = TOOLS

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION, **backend_for({}).fingerprint()},
                "system": {"kind": "simulator", "product": "Discrete-event plant model (open source)",
                           "stands_in_for": "Rockwell Arena Simulation, Emulate3D", "simulated": True}}

    def new_state(self, config):
        missing = [k for k in ("seed", "lines", "decisions", "safety", "study") if k not in config]
        if missing:
            raise errors.invalid(f"config is missing {', '.join(missing)}")
        absent = [k for k in STUDY if k not in config["study"]]
        if absent:
            raise errors.invalid(f"config.study is missing {', '.join(absent)}")
        backend_for(config)
        return {"simulations": [], "recommendation": None, "denials": []}

    def on_refused(self, inst, name, args, error) -> dict:
        """A denial is recorded on the state and reported as a `denial` event; other refusals as usual."""
        if not isinstance(error, Denied):
            return super().on_refused(inst, name, args, error)
        record = {"n": len(inst.state["denials"]) + 1, "tool": name, "args": args, **error.body}
        inst.state = {**inst.state, "denials": [*inst.state["denials"], record]}
        return {"error": str(error), "reads": [], "writes": [DENIALS],
                "events": [event("denial", error.text, record)]}

    def docs(self, config, state):
        s = config["study"]
        out = {"study/budget": {"used": len(state["simulations"]), "limit": s["max_simulations"]},
               "recommendation/final": state["recommendation"] or {"submitted": False},
               DENIALS: {"denials": state["denials"]}}
        out.update({f"simulation/{r['simulation']}": r for r in state["simulations"]})
        return out
