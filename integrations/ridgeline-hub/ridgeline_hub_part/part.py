"""The arena-part/1 integration: one World per instance, saved as one JSON file."""
from arena_part import Part, errors, saves, sources

from . import state
from .config import parse_config, source_files, source_hashes
from .tools import run_tool, schemas
from .world import TICK_MS, World

NAME = "ridgeline-hub"
SEED_NOTE = ("accepted and unused: the market has no randomness, so the replay contract holds for any seed")


def check_fingerprint(cfg):
    """The configured source files must be there and be the ones `describe` declared, or replays would
    differ. Checked before importing them, so a refused start leaves nothing loaded."""
    sources.check_unchanged(source_hashes(parse_config({})), source_files(cfg), "check hub_path and data_path")


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
            return state.read(saves.root(NAME) / target["save"])
        raise errors.invalid("invalid params: target needs instance or save")

    def describe(self):
        return {
            "name": NAME, "version": "0.1.0", "protocol": 1,
            "restore": {"exact": "save", "approximate": False},
            "system": {"kind": "logistics", "product": "Ridgeline hub (simulated suppliers, carriers, customers)", "simulated": True},
            "uses_clock": True,
            "fingerprint": {"adapter": 1, "tick_ms": TICK_MS, "seed": SEED_NOTE,
                            "sha256": source_hashes(parse_config({}))},
            "tools": schemas(),
        }

    def start(self, instance, config, seed, clock_ms, from_=None):
        cfg = parse_config(config)
        check_fingerprint(cfg)
        world = World(cfg, clock_ms)
        if from_ is not None:  # exact saves also serve approximate restores
            if not isinstance(from_, dict) or "save" not in from_:
                raise errors.invalid("invalid params: from needs a save")
            state.restore(world, state.read(saves.root(NAME) / from_["save"]))
        self.instances[instance] = world

    def call_tool(self, instance, name, args, clock_ms):
        world = self._world(instance)
        world.step_to(clock_ms)
        return run_tool(world, name, args)

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
        saves.drop(NAME, save)

    def stop(self, instance):
        self.instances.pop(instance, None)

    def usage(self):
        return {"bytes": 0}  # every save lives under ARENA_SAVES, which arenad measures itself

    def gc(self, keep):
        return saves.gc(NAME, keep)
