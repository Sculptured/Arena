"""Example part: one integer per instance. Mirrors kits/rust/examples/counter.rs.

`python counter.py --flavour <name>` picks how it restores (default "exact"):
- exact: exact save, no replay.
- replay_only: replay only; `save` fails with -32001.
- approx_and_replay: replay, plus approximate saves (reported `exact: false`).
- broken_restore: exact save, but a restore from a save gives 0 (a bug on purpose).
- nondeterministic: replay, but `add` adds a random 0..1000 from the OS (a bug on purpose).
"""
import json
import os
import shutil
import sys
from enum import Enum
from pathlib import Path

from arena_part import Part, PartError, cant_restore, serve, unknown
from arena_part.errors import invalid

KEY = "counter/value"


class Flavour(Enum):
    EXACT = "exact"
    REPLAY_ONLY = "replay_only"
    APPROX_AND_REPLAY = "approx_and_replay"
    BROKEN_RESTORE = "broken_restore"
    NONDETERMINISTIC = "nondeterministic"

    @staticmethod
    def parse(s: str) -> "Flavour":
        try:
            return Flavour(s)
        except ValueError:
            raise invalid(f"unknown flavour: {s}") from None


def saves_root() -> Path:
    """Saves live at `$ARENA_SAVES/counter-py/<save>/` by convention (the package name)."""
    return Path(os.environ.get("ARENA_SAVES", "saves")) / "counter-py"


def read_save(save: str) -> dict:
    try:
        v = json.loads((saves_root() / save / "state.json").read_bytes())
    except OSError:
        raise unknown(f"unknown save {save}") from None
    return {"value": int(v.get("value", 0)), "secs": int(v.get("secs", 0))}


def dir_bytes(d: Path) -> int:
    return sum(f.stat().st_size for f in d.rglob("*") if f.is_file()) if d.is_dir() else 0


def tool(name, description, props, effect):
    return {
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": props},
        "effect": effect,
    }


RESTORE = {
    Flavour.EXACT: {"exact": "save", "approximate": False},
    Flavour.BROKEN_RESTORE: {"exact": "save", "approximate": False},
    Flavour.REPLAY_ONLY: {"exact": "replay", "approximate": False},
    Flavour.NONDETERMINISTIC: {"exact": "replay", "approximate": False},
    Flavour.APPROX_AND_REPLAY: {"exact": "replay", "approximate": True},
}


class Counter(Part):
    def __init__(self, flavour: Flavour = Flavour.EXACT):
        self.flavour = flavour
        self.instances: dict[str, dict] = {}

    def _inst(self, instance):
        try:
            return self.instances[instance]
        except KeyError:
            raise unknown(f"unknown instance {instance}") from None

    def _value_of(self, t) -> int:
        if "instance" in t:
            return self._inst(t["instance"])["value"]
        if "save" in t:
            return read_save(t["save"])["value"]
        raise invalid("invalid params: target needs instance or save")

    def describe(self):
        return {
            "name": "counter-py",
            "version": "0.1.0",
            "protocol": 1,
            "restore": RESTORE[self.flavour],
            "uses_clock": True,
            "fingerprint": {"example": "counter"},
            "tools": [
                tool("add", "Add n to the counter.", {"n": {"type": "integer"}}, "write"),
                tool("get", "Read the counter.", {}, "read"),
            ],
        }

    def start(self, instance, config, seed, clock_ms, from_=None):
        state = {"value": 0, "secs": clock_ms // 1000}
        if from_ is not None:
            f = self.flavour
            exact_ok = f in (Flavour.EXACT, Flavour.BROKEN_RESTORE)
            approx_ok = f == Flavour.APPROX_AND_REPLAY
            ok = exact_ok if from_["mode"] == "exact" else (exact_ok or approx_ok)
            if not ok:
                raise cant_restore("this flavour can't restore from a save in that mode")
            state = read_save(from_["save"])
            if f == Flavour.BROKEN_RESTORE:
                state["value"] = 0
        self.instances[instance] = state

    def call_tool(self, instance, name, args, clock_ms):
        inst = self._inst(instance)
        if name == "add":
            n = args.get("n")
            if not isinstance(n, int) or isinstance(n, bool):
                return {"error": "n must be an integer", "reads": [], "writes": []}
            extra = 0
            if self.flavour == Flavour.NONDETERMINISTIC:
                extra = int.from_bytes(os.urandom(4), "little") % 1000
            inst["value"] += n + extra
            return {"result": {"value": inst["value"]}, "reads": [], "writes": [KEY]}
        if name == "get":
            return {"result": {"value": inst["value"]}, "reads": [KEY], "writes": []}
        return {"error": f"unknown tool {name}", "reads": [], "writes": []}

    def advance(self, instance, clock_ms):
        inst = self._inst(instance)
        secs = clock_ms // 1000
        d = secs - inst["secs"]
        if d > 0:
            inst["value"] += d
            inst["secs"] = secs
        return {"changed": d > 0, "events": []}

    def save(self, instance, save, dir):
        state = self._inst(instance)
        if self.flavour == Flavour.REPLAY_ONLY:
            raise cant_restore("this flavour has no saves")
        body = json.dumps(
            {"value": state["value"], "secs": state["secs"]}, separators=(",", ":")
        ).encode()
        Path(dir).mkdir(parents=True, exist_ok=True)
        (Path(dir) / "state.json").write_bytes(body)
        return {"bytes": len(body), "exact": self.flavour != Flavour.APPROX_AND_REPLAY}

    def inspect(self, target, keys):
        v = self._value_of(target)
        docs = {KEY: v} if not keys else {}
        for k in keys:
            docs[k] = v if k == KEY else None
        return {"docs": docs}

    def diff(self, a, b):
        return {"differing": [] if self._value_of(a) == self._value_of(b) else [KEY]}

    def drop_save(self, save):
        shutil.rmtree(saves_root() / save, ignore_errors=True)

    def stop(self, instance):
        self.instances.pop(instance, None)

    def usage(self):
        return {"bytes": dir_bytes(saves_root())}


def main(argv):
    flavour = Flavour.EXACT
    if "--flavour" in argv:
        i = argv.index("--flavour")
        try:
            flavour = Flavour.parse(argv[i + 1] if i + 1 < len(argv) else "")
        except PartError as e:
            print(e.message, file=sys.stderr)
            return 2
    serve(Counter(flavour))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
