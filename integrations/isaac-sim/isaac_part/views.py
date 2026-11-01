"""The factory's /state and events as the arena sees them: keyed documents, world events, how documents compare,
and the documents file of a save. Shared by the plant and the mirror (GET /state has Factory.state()'s shape)."""
import json
from pathlib import Path

from arena_part import errors

DOCS_FILE = "docs.json"
# key kind -> the /state collection it comes from; each item has an `id`
COLLECTIONS = {"machine": "machines", "line": "lines", "order": "orders", "technician": "technicians",
               "operator": "operators", "shipment": "shipments", "robot": "robots", "incident": "incidents"}
CLOCK_FIELDS = ("t", "clock", "speed", "scenario", "log_seq")
# Floats compare equal within ABS_TOL + REL_TOL * the larger: physics readings after an approximate restore
# (the protocol has no "not compared" mark).
ABS_TOL, REL_TOL = 0.05, 0.01


def plain(value):
    """A JSON-safe copy (datetimes and other objects as strings, as the factory API sends them)."""
    return json.loads(json.dumps(value, default=str))


def docs_of(state: dict) -> dict:
    """Every key of a /state, `<kind>/<id>`, with its document; plus `plant/clock` and `plant/sim`."""
    docs = {f"{kind}/{d['id']}": d for kind, field in COLLECTIONS.items() for d in state.get(field) or []}
    docs["plant/clock"] = {k: state.get(k) for k in CLOCK_FIELDS}
    docs["plant/sim"] = state.get("sim")
    return plain(docs)


def pick(docs: dict, keys) -> dict:
    """inspect's answer: the asked keys (null when absent), or every key when none are asked."""
    return {k: docs.get(k) for k in keys} if keys else docs


def arena_event(ev: dict) -> dict:
    """A factory event as a world event: {kind, text, data}, with source, seq and the plant time t in data."""
    data = {**(ev.get("data") or {}), "source": ev["source"], "seq": ev["seq"], "t": ev["t"]}
    return plain({"kind": ev["kind"], "text": ev["text"], "data": data})


def _number(v) -> bool:
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def same(a, b) -> bool:
    if isinstance(a, float) or isinstance(b, float):
        return _number(a) and _number(b) and abs(a - b) <= ABS_TOL + REL_TOL * max(abs(a), abs(b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(map(same, a, b))
    return a == b


def differing(a: dict, b: dict) -> list:
    return sorted(k for k in a.keys() | b.keys() if not same(a.get(k), b.get(k)))


def write_docs(dir, docs: dict) -> int:
    body = json.dumps(docs, separators=(",", ":"), sort_keys=True).encode()
    Path(dir).mkdir(parents=True, exist_ok=True)
    (Path(dir) / DOCS_FILE).write_bytes(body)
    return len(body)


def read_docs(save_dir) -> dict:
    try:
        return json.loads((Path(save_dir) / DOCS_FILE).read_bytes())
    except OSError:
        raise errors.unknown(f"unknown save {Path(save_dir).name}") from None
