"""The arena-part/1 integration: one World per instance, saved as one JSON file."""
import os
import shutil
from pathlib import Path

from arena_part import Part, errors, kept

from . import state
from .config import parse_config
from .tools import TOOLS, run_tool
from .world import TICK_MS, World

NAME = "ridgeline-hub"
SEED_NOTE = ("accepted and unused: the market has no randomness, so the replay contract holds for any seed")


def saves_root() -> Path:
    """Saves live at `$ARENA_SAVES/ridgeline-hub/<save>/` by convention (the package name)."""
    return Path(os.environ.get("ARENA_SAVES", "saves")) / NAME


class RidgelineHub(Part):
    def __init__(self):
        self.instances: dict[str, World] = {}

    def _world(self, instance) -> World:
        try:
            return self.instances[instance]
        except KeyError:
            raise errors.unknown(f"unknown instance {instance}") from None

    def _snapshot_of(self, target) -> dict:
        if "instance" in target:
            return state.snapshot(self._world(target["instance"]))
        if "save" in target:
            return state.read(saves_root() / target["save"])
        raise errors.invalid("invalid params: target needs instance or save")

    def describe(self):
        return {
            "name": NAME, "version": "0.1.0", "protocol": 1,
            "restore": {"exact": "save", "approximate": False},
            "uses_clock": True,
            "fingerprint": {"market": "ridgeline_hub.market", "adapter": 1, "tick_ms": TICK_MS,
                            "seed": SEED_NOTE},
            "tools": [t.schema() for t in TOOLS.values()],
        }

    def start(self, instance, config, seed, clock_ms, from_=None):
        world = World(parse_config(config), clock_ms)
        if from_ is not None:  # exact saves also serve approximate restores
            state.restore(world, state.read(saves_root() / from_["save"]))
        self.instances[instance] = world

    def call_tool(self, instance, name, args, clock_ms):
        world = self._world(instance)
        world.step_to(clock_ms)
        result, reads, writes = run_tool(world, name, args)
        return {"result": result, "reads": reads, "writes": writes}

    def advance(self, instance, clock_ms):
        events = self._world(instance).advance(clock_ms)
        return {"changed": bool(events), "events": events}

    def save(self, instance, save, dir):
        size = state.write(dir, state.snapshot(self._world(instance)))
        return {"bytes": size, "exact": True}

    def inspect(self, target, keys):
        docs = state.docs_of(self._snapshot_of(target))
        return {"docs": {k: docs.get(k) for k in keys} if keys else docs}

    def diff(self, a, b):
        da, db = state.docs_of(self._snapshot_of(a)), state.docs_of(self._snapshot_of(b))
        return {"differing": sorted(k for k in da.keys() | db.keys() if da.get(k) != db.get(k))}

    def drop_save(self, save):
        shutil.rmtree(saves_root() / save, ignore_errors=True)

    def stop(self, instance):
        self.instances.pop(instance, None)

    def usage(self):
        return {"bytes": 0}  # every save lives under ARENA_SAVES, which arenad measures itself

    def gc(self, keep):
        root = saves_root()
        doomed = [p for p in root.iterdir() if p.is_dir() and not kept(keep, p.name)] if root.is_dir() else []
        freed = sum(f.stat().st_size for p in doomed for f in p.rglob("*") if f.is_file())
        for p in doomed:
            shutil.rmtree(p, ignore_errors=True)
        return {"dropped": len(doomed), "bytes": freed}
