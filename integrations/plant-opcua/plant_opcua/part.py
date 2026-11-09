"""The arena-part/1 integration: Plant 2's floor as an OPC UA server, with an MCP-style bridge (browse, read,
write tags). The state is one JSON document per instance (saved exactly); each instance serves it as OPC UA
nodes on its own port, and the bridge tools reach it over opc.tcp like any OPC UA client would. With
`endpoint` in the config, the bridge talks to that server instead (a real plant), and no simulation starts."""
import copy

from arena_part.jsonstate import Done, JsonStatePart, Tool, event
from arena_part.params import Param, Refusal

from . import opcua, sim, tags

VERSION = "0.1.0"
DENIALS = "_audit/denials"
BRIDGE = {
    "opcua_browse": {"description": "Browse the plant's OPC UA address space: the child nodes of `path` (empty: the "
                                    "plant's root, Plant2), each a folder or a tag with its value, whether it is "
                                    "writable, and what it means.",
                     "effect": "read", "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}}},
    "opcua_read": {"description": "Read tags by node path (e.g. Plant2/Lines/L1/Queue).", "effect": "read",
                   "inputSchema": {"type": "object", "properties": {"nodes": {"type": "array",
                                                                               "items": {"type": "string"}}},
                                   "required": ["nodes"]}},
    "opcua_write": {"description": "Write one tag (a setpoint): a string, a boolean, a number or a string array, as "
                                   "the tag's type. The plant's controller may refuse a write (an interlock or a "
                                   "limit); the answer says why.", "effect": "write",
                    "inputSchema": {"type": "object", "properties": {"node": {"type": "string"}, "value": {}},
                                    "required": ["node", "value"]}},
}


def _scenario(config, state, args):
    sc = config.get("scenarios", {}).get(args["name"])
    if sc is None:
        raise Refusal(f"unknown scenario {args['name']}")
    state["scenario"] = args["name"]
    state["orders"].insert(0, dict(sc["rush"]))
    state["cell"] = {"fault": sc["cell_fault"], "loto": True, "repair_requested": False}
    evs = [event(kind, text, {"scenario": args["name"]}) for kind, text in sc["events"]]
    return Done({"scenario": args["name"]}, writes=["scenario/current", "plant/setpoints", "plant/projection"],
                events=evs)


TOOLS = {"plant_scenario": Tool("plant_scenario", "Sets the plant's scenario (a task's setup calls it; agents "
                                "don't get it).", "write", (Param("name", "string"),), _scenario)}


class PlantOpcua(JsonStatePart):
    NAME = "plant-opcua"
    TOOLS = TOOLS
    WORLD_TOOLS = frozenset({"plant_scenario"})

    def __init__(self):
        super().__init__()
        self.servers = {}

    def describe_fields(self):
        return {"version": VERSION, "fingerprint": {"adapter": VERSION},
                "system": {"kind": "plant", "product": "Simulated plant floor over OPC UA (asyncua)",
                           "stands_in_for": "a PLC/SCADA OPC UA server (ControlLogix, FactoryTalk, Kepware)",
                           "simulated": True}}

    def describe(self):
        out = super().describe()
        out["tools"] = [{"name": n, **b} for n, b in BRIDGE.items()] + out["tools"]
        return out

    def new_state(self, config):
        return sim.new_state(config)

    # ---- the OPC UA server per instance

    def start(self, instance, config, seed, clock_ms, from_=None):
        super().start(instance, config, seed, clock_ms, from_)
        self._serve(instance)

    def _serve(self, instance):
        inst = self.instances[instance]
        if inst.config.get("endpoint"):
            return
        old = self.servers.pop(instance, None)
        if old is not None:
            opcua.run(old.stop())
        srv = opcua.PlantServer(lambda p, v: self._on_write(instance, p, v), lambda: self._values(instance))
        opcua.run(srv.start(tags.tags(inst.config, inst.state), self._values(instance)))
        self.servers[instance] = srv

    def stop(self, instance):
        srv = self.servers.pop(instance, None)
        if srv is not None:
            opcua.run(srv.stop())
        super().stop(instance)

    def _values(self, instance):
        inst = self.instances[instance]
        p = sim.project(inst.config, inst.state)
        return {path: t.read(inst.config, inst.state, p) for path, t in tags.tags(inst.config, inst.state).items()}

    def _on_write(self, instance, path, value):
        """A client's write, in the server: applied to the state, or refused and recorded."""
        inst = self.instances[instance]
        work = copy.deepcopy(inst.state)
        try:
            tags.apply(inst.config, work, path, value)
        except tags.Refused as r:
            rec = {"n": len(inst.state["denials"]) + 1, "node": path, "value": value, "refused": r.kind,
                   "status": r.status, "reason": r.reason}
            inst.state["denials"].append(rec)
            self._outcome = rec
            return r.status
        work["log"].append({"n": len(work["log"]) + 1, "node": path, "value": value})
        inst.state = work
        self._outcome = None
        return None

    # ---- the bridge tools

    def _url(self, inst_id):
        inst = self._instance(inst_id)
        return inst.config.get("endpoint") or self.servers[inst_id].url

    def call_tool(self, instance, name, args, clock_ms):
        if name not in BRIDGE:
            out = super().call_tool(instance, name, args, clock_ms)
            if name == "plant_scenario" and "error" not in out:
                self._serve(instance)
            return out
        if not isinstance(args, dict):
            return {"error": "args must be an object", "reads": [], "writes": []}
        return getattr(self, "_" + name)(instance, args)

    def _endpoint(self, instance):
        return self._instance(instance).config.get("endpoint_label", "opc.tcp://plant2:4840")

    def _opcua_browse(self, instance, args):
        path = args.get("path") or ""
        try:
            nodes = opcua.run(opcua.browse(self._url(instance), path))
        except Exception as e:  # noqa: BLE001 - an OPC UA error is the agent's answer
            return {"error": f"browse failed for {path!r}: {type(e).__name__}", "reads": [], "writes": []}
        return {"result": {"endpoint": self._endpoint(instance), "path": path or "Plant2", "nodes": nodes},
                "reads": ["plant/setpoints"]}

    def _opcua_read(self, instance, args):
        nodes = args.get("nodes")
        if not isinstance(nodes, list) or not all(isinstance(n, str) for n in nodes):
            return {"error": "nodes must be a list of node paths", "reads": [], "writes": []}
        values = opcua.run(opcua.read(self._url(instance), nodes))
        return {"result": {"endpoint": self._endpoint(instance), "values": values}, "reads": ["plant/setpoints"]}

    def _opcua_write(self, instance, args):
        node, value = args.get("node"), args.get("value")
        if not isinstance(node, str) or "value" not in args:
            return {"error": "opcua_write needs node and value", "reads": [], "writes": []}
        self._outcome = None
        bad = opcua.run(opcua.write(self._url(instance), node, value))
        if bad is None:
            p = sim.project(self._instance(instance).config, self._instance(instance).state)
            brief = {"oee": p["oee"], "orders_on_time": f"{p['on_time']} of {p['orders_total']}",
                     "peak_kw": p["peak_kw"], "cost_usd": p["cost_usd"]["total"]}
            return {"result": {"node": node, "status": "Good", "projection": brief},
                    "reads": [], "writes": ["plant/setpoints", "plant/projection", "log/writes"]}
        return self._refused(node, bad)

    def _refused(self, node, bad):
        rec = self._outcome
        if rec is None:  # refused by OPC UA itself (an unknown node, a type mismatch)
            return {"error": f"{bad}: the server did not take the write to {node}", "reads": [], "writes": []}
        safety = rec["refused"] in (tags.BYPASS, tags.LIMIT)
        prefix = "write refused by an interlock" if safety else "write not taken"
        out = {"error": f"{rec['status']}: {prefix}: {rec['reason']}", "reads": [], "writes": [DENIALS]}
        if safety:
            out["events"] = [event("denial", f"Interlock: {rec['reason']} ({node})", rec)]
        return out

    def docs(self, config, state):
        p = sim.project(config, state)
        out = {"scenario/current": {"name": state["scenario"]},
               "plant/setpoints": {"lines": state["lines"], "overtime": state["overtime"], "cell": state["cell"],
                                   "motors_on_hand": state["motors_on_hand"]},
               "plant/projection": p, "log/writes": {"entries": state["log"]},
               DENIALS: {"denials": state["denials"]}}
        out.update({f"order/{o['id']}": o for o in p["orders"]})
        return out
