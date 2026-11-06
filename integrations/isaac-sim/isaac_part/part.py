"""The arena-part/1 integration: one Plant per instance; exact by replay, approximate from a save."""
import importlib

from arena_part import errors, saves

from . import recording, showcase_layout, source, state, views
from .base import DocsPart
from .config import parse_config
from .plant import Plant, physics_hz
from .tools import TOOLS, run_tool

NAME = "isaac-sim"
ADAPTER = 1


def _camera_names(cfg):
    """The shots a recording may stay on: the factory's rigs, or the showcase's."""
    layout = importlib.import_module("layout")
    return showcase_layout.shots(layout) if cfg.scene == "showcase" else layout.CAMERAS


class IsaacSim(DocsPart):
    name = NAME

    def __init__(self, backend):
        super().__init__()
        self.backend = backend

    def describe(self):
        return {
            "name": NAME, "version": "0.1.0", "protocol": 1,
            "restore": {"exact": "replay", "approximate": True},
            "system": {"kind": "plant", "product": "NVIDIA Isaac Sim", "simulated": True},
            "uses_clock": True,
            "fingerprint": {"adapter": ADAPTER, "adapter_sha256": source.adapter_hashes(), "backend": self.backend.name,
                            "physics_hz": source.default_physics_hz(), "sha256": source.declared(self.backend),
                            **self.backend.host()},
            "tools": [t.schema() for t in TOOLS.values()],
        }

    def _check_from(self, from_) -> None:
        """A start `from` a save must name one and be approximate: exact restores are by replay only."""
        if from_ is None:
            return
        if not isinstance(from_, dict) or "save" not in from_ or from_.get("mode") not in ("exact", "approximate"):
            raise errors.invalid("invalid params: from needs a save and a mode")
        if from_["mode"] == "exact":
            raise errors.cant_restore("isaac-sim restores exactly only by replay (physics state on the GPU can't "
                                      "be saved exactly); restore approximately instead")

    def _check_room(self, instance):
        live = [i for i in self.instances if i != instance]
        cap = self.backend.max_live
        if cap is not None and len(live) >= cap:
            raise errors.internal(f"the {self.backend.name} backend runs {cap} plant per process; stop {live[0]} "
                                  "first")

    def _check_record(self, cfg):
        if cfg.record is not None and self.backend.video is None:
            raise errors.invalid(f"record needs the isaac backend (Isaac Sim's RTX renders the frames); the "
                                 f"{self.backend.name} backend can't render")

    def start(self, instance, config, seed, clock_ms, from_=None):
        cfg = parse_config(config)
        self._check_from(from_)
        self._check_room(instance)
        source.check(cfg, self.backend)
        self._check_record(cfg)
        mods = source.load(cfg)
        if cfg.record is not None:
            recording.check_camera(cfg.camera, _camera_names(cfg))
        self.backend.load(cfg)  # the factory's and the backend's modules come before any unpickling
        snap = None
        if from_ is not None:
            snap = state.read(saves.root(NAME) / from_["save"])
            state.check_made_with(snap, state.made_with(self.backend, physics_hz(cfg, mods), cfg))
        video = None if cfg.record is None else recording.path_for(cfg.record, saves.root(NAME), instance)
        plant = Plant(mods, self.backend, cfg, clock_ms, video)
        if snap is None:
            plant.begin(seed, cfg.scenario)
        else:
            state.restore(plant, snap)
        self.instances[instance] = plant

    def stop(self, instance):
        plant = self.instances.pop(instance, None)
        if plant is not None:
            plant.stop()

    def call_tool(self, instance, name, args, clock_ms):
        plant = self._instance(instance)
        plant.step_to(clock_ms)
        return run_tool(plant, name, args)

    def advance(self, instance, clock_ms):
        return self._instance(instance).advance(clock_ms)

    def save(self, instance, save, dir):
        """Written to `$ARENA_SAVES/isaac-sim/<save>`, which is `dir` when arenad runs this program itself. When it
        runs on another machine (ARENA_RUN_ISAAC_SIM), `dir` is arenad's path and the saves stay on this one."""
        del dir
        return {"bytes": state.write(saves.root(NAME) / save, self._instance(instance)), "exact": False}

    def _live_docs(self, instance) -> dict:
        return views.docs_of(self._instance(instance).state())
