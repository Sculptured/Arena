"""The arena-part/1 integration: one Plant per instance; exact by replay, approximate from a save."""
from arena_part import errors, saves

from . import source, state, views
from .base import DocsPart
from .config import parse_config
from .plant import Plant
from .tools import TOOLS, run_tool

NAME = "isaac-sim"
ADAPTER = 1


class IsaacSim(DocsPart):
    name = NAME

    def __init__(self, backend):
        super().__init__()
        self.backend = backend

    def describe(self):
        return {
            "name": NAME, "version": "0.1.0", "protocol": 1,
            "restore": {"exact": "replay", "approximate": True},
            "uses_clock": True,
            "fingerprint": {"adapter": ADAPTER, "backend": self.backend.name,
                            "physics_hz": source.default_physics_hz(), "sha256": source.declared(self.backend),
                            **self.backend.host()},
            "tools": [t.schema() for t in TOOLS.values()],
        }

    def _restore_point(self, from_):
        """The snapshot to start from, or None. Exact restores are by replay only."""
        if from_ is None:
            return None
        if not isinstance(from_, dict) or "save" not in from_ or from_.get("mode") not in ("exact", "approximate"):
            raise errors.invalid("invalid params: from needs a save and a mode")
        if from_["mode"] == "exact":
            raise errors.cant_restore("isaac-sim restores exactly only by replay (physics state on the GPU can't "
                                      "be saved exactly); restore approximately instead")
        return state.read(saves.root(NAME) / from_["save"])

    def _check_room(self, instance):
        live = [i for i in self.instances if i != instance]
        cap = self.backend.max_live
        if cap is not None and len(live) >= cap:
            raise errors.internal(f"the {self.backend.name} backend runs {cap} plant per process; stop {live[0]} "
                                  "first")

    def start(self, instance, config, seed, clock_ms, from_=None):
        cfg = parse_config(config)
        snap = self._restore_point(from_)
        self._check_room(instance)
        source.check(cfg, self.backend)
        plant = Plant(source.load(cfg), self.backend, cfg, clock_ms)
        if snap is None:
            plant.begin(seed, cfg.scenario)
        else:
            state.restore(plant, snap)
        self.instances[instance] = plant

    def call_tool(self, instance, name, args, clock_ms):
        plant = self._instance(instance)
        plant.step_to(clock_ms)
        return run_tool(plant, name, args)

    def advance(self, instance, clock_ms):
        return self._instance(instance).advance(clock_ms)

    def save(self, instance, save, dir):
        return {"bytes": state.write(dir, self._instance(instance)), "exact": False}

    def _live_docs(self, instance) -> dict:
        return views.docs_of(self._instance(instance).state())
