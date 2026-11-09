"""A part whose instance is a config plus one JSON document of state, with no clock: exact saves as one
`state.json`, keyed documents for `inspect` and `diff`, and a table of tools that each either change the
state or refuse and change nothing.

Subclass `JsonStatePart` and give it `NAME` (the package name, which names its save folders), `TOOLS`
(`{name: Tool}`), `describe_fields()` (`version`, `fingerprint`, optionally `system`), `new_state(config)`
(a fresh state; validate the config there) and `docs(config, state)` (`{key: doc}`).

A part with scheduled disruptions (`disruptions`) also gives `DISRUPTIONS` (`{do.kind: Kind}`) and
`WORLD_TOOLS` (the tools a task's setup or a link calls: they are not agents' calls, so they count
for no disruption). Its scenario tool arms them with `disruptions.arm`.
"""
import copy
import json
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from . import disruptions, errors, saves
from .params import Refusal, input_schema, normalize, refused
from .serve import Part

FILE = "state.json"


@dataclass
class Done:
    """What a tool did: its result, the keys it read and wrote, and the events it gives the world."""
    result: object
    reads: list = field(default_factory=list)
    writes: list = field(default_factory=list)
    events: list = field(default_factory=list)


@dataclass(frozen=True)
class Tool:
    name: str
    description: str
    effect: str  # "read" or "write"
    params: tuple  # of params.Param
    run: Callable  # (config, state, args) -> Done; raises Refusal to refuse

    def schema(self) -> dict:
        return {"name": self.name, "description": self.description, "effect": self.effect,
                "inputSchema": input_schema(self.params)}


@dataclass
class Instance:
    config: dict
    state: dict


def event(kind: str, text: str, data: dict) -> dict:
    """A world event for `Done.events`."""
    return {"kind": kind, "text": text, "data": data}


class JsonStatePart(Part):
    NAME = ""
    TOOLS: dict = {}
    DISRUPTIONS: dict = {}
    WORLD_TOOLS: frozenset = frozenset()

    def __init__(self):
        self.instances: dict[str, Instance] = {}

    def describe_fields(self) -> dict:
        raise NotImplementedError("describe_fields")

    def new_state(self, config) -> dict:
        raise NotImplementedError("new_state")

    def docs(self, config, state) -> dict:
        raise NotImplementedError("docs")

    def describe(self):
        fixed = {"name": self.NAME, "protocol": 1, "restore": {"exact": "save", "approximate": False},
                 "uses_clock": False}
        return {**fixed, **self.describe_fields(), "tools": [t.schema() for t in self.TOOLS.values()]}

    def _instance(self, instance) -> Instance:
        try:
            return self.instances[instance]
        except KeyError:
            raise errors.unknown(f"unknown instance {instance}") from None

    def start(self, instance, config, seed, clock_ms, from_=None):
        config = config or {}
        state = self.new_state(config)
        if from_ is not None:
            if not isinstance(from_, dict) or "save" not in from_:
                raise errors.invalid("invalid params: from needs a save")
            state = _read(saves.root(self.NAME) / from_["save"])["state"]
        self.instances[instance] = Instance(config, state)

    def call_tool(self, instance, name, args, clock_ms):
        """Runs the tool on a copy of the state and keeps the copy only when the tool did not refuse.
        The disruptions an agent's call sets off fire first and stay, even when the tool refuses."""
        inst = self._instance(instance)
        tool = self.TOOLS.get(name)
        if tool is None:
            raise errors.invalid(f"unknown tool {name}")
        before = self._scheduled(inst, name)
        work = copy.deepcopy(inst.state)
        try:
            done = tool.run(inst.config, work, normalize(name, tool.params, args))
        except (Refusal, ValueError) as e:
            return disruptions.merged(self.on_refused(inst, name, args, e), before)
        inst.state = work
        out = {"result": done.result, "reads": done.reads, "writes": done.writes}
        if done.events:
            out["events"] = done.events
        return disruptions.merged(out, before)

    def on_refused(self, inst, name, args, error) -> dict:
        """The answer to a call the tool refused. A part that records refusals (a denial by a hard
        limit) overrides it: it may change `inst.state`, name the keys in `writes` and give `events`."""
        return refused(str(error))

    def _scheduled(self, inst, name) -> disruptions.Fired:
        """Fires and counts the disruptions an agent's call of `name` sets off, on the state itself."""
        if name in self.WORLD_TOOLS or not disruptions.pending(inst.state, name):
            return disruptions.Fired()
        state = copy.deepcopy(inst.state)
        fired = disruptions.on_call(inst.config, state, self.DISRUPTIONS, name)
        inst.state = state
        return fired

    def save(self, instance, save, dir):
        inst = self._instance(instance)
        body = json.dumps({"config": inst.config, "state": inst.state}, separators=(",", ":"), sort_keys=True)
        Path(dir).mkdir(parents=True, exist_ok=True)
        (Path(dir) / FILE).write_text(body)
        return {"bytes": len(body.encode()), "exact": True}

    def _docs_of(self, target) -> dict:
        if "instance" in target:
            inst = self._instance(target["instance"])
            return self.docs(inst.config, inst.state)
        if "save" in target:
            saved = _read(saves.root(self.NAME) / target["save"])
            return self.docs(saved["config"], saved["state"])
        raise errors.invalid("invalid params: target needs instance or save")

    def inspect(self, target, keys):
        docs = json.loads(json.dumps(self._docs_of(target)))
        return {"docs": {k: docs.get(k) for k in keys} if keys else docs}

    def diff(self, a, b):
        da, db = self._docs_of(a), self._docs_of(b)
        return {"differing": sorted(k for k in da.keys() | db.keys() if da.get(k) != db.get(k))}

    def drop_save(self, save):
        saves.drop(self.NAME, save)

    def stop(self, instance):
        self.instances.pop(instance, None)

    def usage(self):
        return {"bytes": 0}  # every save lives under ARENA_SAVES, which arenad measures itself

    def gc(self, keep):
        return saves.gc(self.NAME, keep)


def set_scenario(config, state, args, known, overlay, kinds, tools) -> Done:
    """A scenario tool's work: sets `state["scenario"]` to `args["name"]` (one of `known`) and arms
    the disruptions the scenario's data lists (`overlay(config, state)["disruptions"]`), then those
    in `args["disruptions"]`, as `disruptions.arm` does with `kinds` and the part's `tools`."""
    if args["name"] not in known:
        raise Refusal(f"unknown scenario {args['name']} (known: {', '.join(known)})")
    state["scenario"] = args["name"]
    given = [*overlay(config, state).get("disruptions", []), *(args.get("disruptions") or [])]
    if not given:
        return Done({"scenario": args["name"]}, writes=["scenario/current"])
    fired = disruptions.arm(config, state, kinds, list(tools), given)
    result = {"scenario": args["name"], "disruptions": [d["id"] for d in given]}
    return Done(result, writes=["scenario/current", *fired.writes], events=fired.events)



def _read(save_dir) -> dict:
    try:
        return json.loads((Path(save_dir) / FILE).read_text())
    except OSError:
        raise errors.unknown(f"unknown save {Path(save_dir).name}") from None
