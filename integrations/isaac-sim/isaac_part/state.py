"""Approximate saves: the Factory's plain state and the backend's simulation state, pickled and gzipped, with the
documents beside them for inspect and diff. The pickle is read only from this integration's own save folders, and
the fingerprint guards the code that reads it."""
import gzip
import io
import pickle
from pathlib import Path

from arena_part import errors

from . import views

FILE = "plant.pkl.gz"
# Every Factory attribute is in exactly one of these (test_state_whitelist_covers_factory_attributes).
FACTORY_SAVED = (
    "data", "machines_meta", "lines_meta", "params", "render_fps", "real_time_factor", "seq", "events", "erp_ok",
    "rng", "t", "clock", "speed", "scenario", "scenario_t0", "step_n", "presses", "lines", "orders", "order_base",
    "etas", "last_written", "next_writeback", "next_eta", "promised", "technicians", "operators", "shipments",
    "robots", "crate", "truck", "incidents", "alarm_snapshot",
)
FACTORY_EXCLUDED = ("io", "lock", "cond", "inbox", "erp", "triggers", "log")  # rebuilt with the instance, or None


class _Pickler(pickle.Pickler):
    """Saves each press's 120 s spectrum history (signals.Ring, about 0.5 MB a press) empty: no tick reads it, only
    vibration_spectrum does, so after a restore the spectrum history starts again. The pre-trip window
    (VibrationChannel.frozen) is kept."""

    def reducer_override(self, obj):
        if type(obj).__name__ == "Ring" and type(obj).__module__ == "signals":
            return type(obj), obj.data.shape
        return NotImplemented


def made_with(backend, hz: int, cfg) -> dict:
    """What a save must match to be restored: backend, physics rate, plant clock start, data file, the cells and
    the load model."""
    return {"backend": backend.name, "physics_hz": hz, "plant_start": cfg.plant_start.isoformat(),
            "data_path": str(cfg.data_path), "cells": list(cfg.cells), "power": cfg.power}


def snapshot(plant) -> dict:
    f = plant.factory
    return {"made_with": made_with(plant.backend, plant.hz, plant.cfg), "steps": plant.steps,
            "last_seq": plant.last_seq, "seed": plant.seed, "equipment": plant.equipment.snapshot(),
            "factory": {a: getattr(f, a) for a in FACTORY_SAVED}, "sim": plant.backend.get_state(plant.io)}


def _dumps(snap) -> bytes:
    buf = io.BytesIO()
    _Pickler(buf, protocol=5).dump(snap)
    return gzip.compress(buf.getvalue(), compresslevel=6)


def write(dir, plant) -> int:
    """Writes the pickle and the documents into the save folder; returns the bytes written."""
    body = plant.act(lambda _f: _dumps(snapshot(plant)))
    Path(dir).mkdir(parents=True, exist_ok=True)
    (Path(dir) / FILE).write_bytes(body)
    return len(body) + views.write_docs(dir, views.docs_of(plant.state()))


def read(save_dir) -> dict:
    """A save's snapshot. Load the factory's and the backend's modules first: the pickle refers to their classes."""
    try:
        body = (Path(save_dir) / FILE).read_bytes()
    except OSError:
        raise errors.unknown(f"unknown save {Path(save_dir).name}") from None
    try:
        return pickle.loads(gzip.decompress(body))
    except (OSError, EOFError, ImportError, AttributeError, pickle.UnpicklingError) as e:
        raise errors.cant_restore(f"save {Path(save_dir).name} can't be read here: {e!r}") from None


def check_made_with(snap: dict, expected: dict) -> None:
    """-32001 when the save was made with another backend, physics rate, plant start or data file."""
    for key, want in expected.items():
        got = snap["made_with"].get(key)
        if got != want:
            raise errors.cant_restore(f"the save was made with {key} {got}, not {want}")


def restore(plant, snap: dict) -> None:
    """Sets a freshly built plant back to a snapshot (already checked with check_made_with)."""
    def put(f):
        for a in FACTORY_SAVED:
            setattr(f, a, snap["factory"][a])
        plant.backend.set_state(plant.io, snap["sim"], f)
    plant.act(put)
    plant.steps, plant.last_seq, plant.seed = snap["steps"], snap["last_seq"], snap["seed"]
    plant.equipment.restore(snap["equipment"])
