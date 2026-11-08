"""The arena-part/1 integration: a config (the network as designed) and a state (isolations, sessions, incidents,
the log and the schedule) per instance, saved as one JSON file."""
from arena_part import disruptions
from arena_part.jsonstate import JsonStatePart

from . import disrupt, network
from .tools import TOOLS

VERSION = "0.1.0"


class OtNetwork(JsonStatePart):
    NAME = "ot-network"
    TOOLS = TOOLS
    DISRUPTIONS = disrupt.KINDS
    WORLD_TOOLS = frozenset({"ot_scenario"})

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "network", "product": "Simulated OT network and IT/OT remote-access gateway",
                           "stands_in_for": "an industrial firewall and remote-access gateway, an OT monitoring tool",
                           "simulated": True}}

    def new_state(self, config):
        network.check_config(config)
        return network.new_state(config)

    def docs(self, config, state):
        out = {"scenario/current": {"name": state["scenario"]},
               f"gateway/{config['gateway']['id']}": config["gateway"],
               f"playbook/{config['playbook']['id']}": config["playbook"],
               "log/actions": {"entries": state["log"]}}
        out.update({f"zone/{z['id']}": network.zone_doc(config, state, z["id"]) for z in config["zones"]})
        out.update({f"conduit/{c['id']}": network.conduit_doc(state, c) for c in config["conduits"]})
        out.update({f"session/{k}": network.session_doc(state, s) for k, s in state["sessions"].items()})
        out.update({f"incident/{k}": v for k, v in state["incidents"].items()})
        out.update(disruptions.docs(state))
        return out
