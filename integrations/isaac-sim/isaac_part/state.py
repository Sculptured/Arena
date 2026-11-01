"""Approximate saves: the Factory's plain state and the backend's simulation state, pickled, with the documents
beside them for inspect and diff. The pickle is read only from this integration's own save folders, and the
fingerprint guards the code that reads it."""
import pickle
from pathlib import Path

from arena_part import errors

from . import views

FILE = "plant.pkl"
# Every Factory attribute is in exactly one of these (test_state_whitelist_covers_factory_attributes).
FACTORY_SAVED = (
    "data", "machines_meta", "lines_meta", "params", "render_fps", "real_time_factor", "seq", "events", "erp_ok",
    "rng", "t", "clock", "speed", "scenario", "scenario_t0", "step_n", "presses", "lines", "orders", "order_base",
    "etas", "last_written", "next_writeback", "next_eta", "promised", "technicians", "operators", "shipments",
    "robots", "crate", "truck", "incidents", "alarm_snapshot",
)
FACTORY_EXCLUDED = ("io", "lock", "cond", "inbox", "erp", "triggers", "log")  # rebuilt with the instance, or None


def snapshot(plant) -> dict:
    f = plant.factory
    return {"backend": plant.backend.name, "physics_hz": plant.hz, "steps": plant.steps, "last_seq": plant.last_seq,
            "factory": {a: getattr(f, a) for a in FACTORY_SAVED}, "sim": plant.backend.get_state(plant.io)}


def write(dir, plant) -> int:
    """Writes the pickle and the documents into the save folder; returns the bytes written."""
    body = plant.act(lambda _f: pickle.dumps(snapshot(plant), protocol=5))
    Path(dir).mkdir(parents=True, exist_ok=True)
    (Path(dir) / FILE).write_bytes(body)
    return len(body) + views.write_docs(dir, views.docs_of(plant.state()))


def read(save_dir) -> dict:
    try:
        return pickle.loads((Path(save_dir) / FILE).read_bytes())
    except OSError:
        raise errors.unknown(f"unknown save {Path(save_dir).name}") from None


def restore(plant, snap: dict) -> None:
    """Sets a freshly built plant back to a snapshot."""
    made = (snap["backend"], snap["physics_hz"])
    if made != (plant.backend.name, plant.hz):
        raise errors.cant_restore(f"the save was made on backend {made[0]} at {made[1]} Hz, not "
                                  f"{plant.backend.name} at {plant.hz} Hz")

    def put(f):
        for a in FACTORY_SAVED:
            setattr(f, a, snap["factory"][a])
        plant.backend.set_state(plant.io, snap["sim"], f)
    plant.act(put)
    plant.steps, plant.last_seq = snap["steps"], snap["last_seq"]
