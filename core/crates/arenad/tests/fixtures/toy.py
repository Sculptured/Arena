"""Toy arena-part/1 packages for arenad tests.

  toy.py part [--crash-on-call N [--crash-mark FILE]] [--save-fails-after N] [--bad-diff] [--log FILE]
              [--prefix P] [--exact save|replay|none] [--approximate] [--uses-clock [--event-every]]
              [--save-sleep MS] [--refuse-start-if FILE] [--describe-fails] [--describe-hangs] [--gc] [--gc-fails]
              [--gc-waits-for FILE] [--tool-event KIND] [--system JSON]
      An integration with one counter per instance and tools `poke` (write, adds 1) and `peek`
      (read), named `<P>poke` and `<P>peek` with --prefix. It exits on its N-th call_tool (only
      once across restarts when FILE marks that it did); every `save` after the first N is an
      error; --bad-diff answers `diff` without `differing`; --log appends "<method> <instance>"
      per request to FILE. Saves are kept in memory; `drop_save` of an unknown one answers -32002. `inspect` refuses keys `bad/..` (-32602). No
      `connect` (method not found). --exact and --approximate set what `describe` reports (default
      exact save): a part without exact saves or approximate ones can't `save` (-32001), and a
      start from a save in a mode the part lacks is refused (-32001). With --uses-clock, `advance`
      reports a change whenever the clock moved, and an `alarm` event on an instance's first
      advance (on every advance with --event-every). --save-sleep makes every `save` take MS
      milliseconds. With --refuse-start-if, `start` answers -32003 (fingerprint mismatch) while
      FILE exists. With --gc-waits-for, `gc` answers only once FILE exists. With --tool-event,
      `poke`'s result gives an event of that kind with data {"n": <the new value>}.
  toy.py bench --integration NAME [--part NAME=INTEGRATION ...] [--gold TOOL [--gold-args JSON]]
               [--config PART=JSON ...] [--clash] [--bench-tool [--tool-part PART]] [--link JSON ...]
               [--gate JSON ...] [--setup JSON ...] [--scored [--two-tasks] [--safety]]
      A benchmark with one task `t1` whose world is one part `box` of integration NAME, or the
      --part list; each part's config is `{}` or the --config given for it. Tools: none; with --clash a `poke`; with --bench-tool a `ring` that reports it
      ran on PART (default `box`). --link adds a link to the world, --gate a gate, --setup a call to `t1`'s setup. With --scored,
      `score` has one check `poked`: the first part's box/n is at least 1. Gold: one TOOL with JSON args (default `poke` with `{}`).
      --two-tasks adds task t2, which needs two pokes (`poked`: box/n is at least the task's number).
      --safety adds the safety check `not_overpoked`: box/n is at most the task's number. --system
      makes the part's `describe` say what kind of system it is (JSON).
"""
import json
import os
import sys
import time

ARGS = sys.argv[1:]


class RpcError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code, self.message = code, message


def opt(name, default):
    return ARGS[ARGS.index(name) + 1] if name in ARGS else default


def opts(name):
    return [ARGS[i + 1] for i, a in enumerate(ARGS) if a == name and i + 1 < len(ARGS)]


PREFIX = opt("--prefix", "")
TOOLS = [
    {"name": PREFIX + "poke", "description": "Add one.", "inputSchema": {"type": "object", "properties": {}}, "effect": "write"},
    {"name": PREFIX + "peek", "description": "Read the counter.", "inputSchema": {"type": "object", "properties": {}}, "effect": "read"},
]
EXACT = opt("--exact", "save")
APPROXIMATE = "--approximate" in ARGS


class Part:
    def __init__(self):
        self.crash_on = int(opt("--crash-on-call", "0"))
        self.saves_ok = int(opt("--save-fails-after", "-1"))
        self.calls = self.saved = 0
        self.counters, self.saves, self.clocks = {}, {}, {}

    def describe(self, p):
        if "--describe-hangs" in ARGS:
            time.sleep(60)
        if "--describe-fails" in ARGS:
            raise RpcError(-32000, "the toy cannot describe itself")
        return {"name": "toy", "version": "0.1.0", "protocol": 1,
                "restore": {"exact": EXACT, "approximate": APPROXIMATE}, "uses_clock": "--uses-clock" in ARGS,
                "fingerprint": {"toy": 1}, "tools": TOOLS, **system()}

    def value(self, target):
        if "save" in target:
            return self.lookup(self.saves, target["save"], "save")
        return self.lookup(self.counters, target["instance"], "instance")

    @staticmethod
    def lookup(table, key, what):
        if key not in table:
            raise RpcError(-32002, "unknown %s %s" % (what, key))
        return table[key]

    def start(self, p):
        refuse = opt("--refuse-start-if", None)
        if refuse and os.path.exists(refuse):
            raise RpcError(-32003, "the toy moved to another machine")
        src = p.get("from") or {}
        mode_ok = EXACT == "save" or (src.get("mode") == "approximate" and APPROXIMATE)
        if src and not mode_ok:
            raise RpcError(-32001, "the toy can't restore from a save in that mode")
        self.counters[p["instance"]] = 0 if not src else self.lookup(self.saves, src["save"], "save")
        return {}

    def advance(self, p):
        i, t = p["instance"], p["clock_ms"]
        self.lookup(self.counters, i, "instance")
        first = i not in self.clocks
        changed = t > self.clocks.get(i, 0)
        self.clocks[i] = max(t, self.clocks.get(i, 0))
        wake = first or "--event-every" in ARGS
        events = [{"kind": "alarm", "text": "the toy woke up", "data": {"at": t}}] if wake else []
        return {"changed": changed, "events": events}

    def call_tool(self, p):
        self.calls += 1
        if self.crash_on and self.calls >= self.crash_on:
            self.crash()
        i = p["instance"]
        n = self.lookup(self.counters, i, "instance")
        if p["name"] == PREFIX + "poke":
            self.counters[i] = n + 1
            out = {"result": {"n": n + 1}, "reads": [], "writes": ["box/n"]}
            if "--tool-event" in ARGS:
                out["events"] = [{"kind": opt("--tool-event", ""), "text": "poked", "data": {"n": n + 1}}]
            return out
        if p["name"] == PREFIX + "peek":
            return {"result": {"n": n}, "reads": ["box/n"], "writes": []}
        return {"error": "unknown tool " + p["name"], "reads": [], "writes": []}

    def crash(self):
        mark = opt("--crash-mark", None)
        if mark and os.path.exists(mark):
            return
        if mark:
            open(mark, "w").close()
        sys.stderr.write("toy: crashing on call %d\n" % self.calls)
        sys.stderr.flush()
        sys.exit(7)

    def save(self, p):
        if EXACT != "save" and not APPROXIMATE:
            raise RpcError(-32001, "the toy has no saves")
        time.sleep(int(opt("--save-sleep", "0")) / 1000)
        self.saved += 1
        if 0 <= self.saves_ok < self.saved:
            raise RpcError(-32000, "the toy can't save today")
        self.saves[p["save"]] = self.lookup(self.counters, p["instance"], "instance")
        return {"bytes": 1, "exact": EXACT == "save"}

    def inspect(self, p):
        v = self.value(p["target"])
        keys = p.get("keys") or ["box/n"]
        if any(k.startswith("bad/") for k in keys):
            raise RpcError(-32602, "the toy has no bad keys")
        return {"docs": {k: ({"n": v} if k == "box/n" else None) for k in keys}}

    def diff(self, p):
        if "--bad-diff" in ARGS:
            return {"changed": True}
        return {"differing": [] if self.value(p["a"]) == self.value(p["b"]) else ["box/n"]}

    def drop_save(self, p):
        self.lookup(self.saves, p["save"], "save")
        del self.saves[p["save"]]
        return {}

    def stop(self, p):
        self.counters.pop(p["instance"], None)
        return {}

    def usage(self, p):
        return {"bytes": len(self.saves)}

    def gc(self, p):
        if "--gc" not in ARGS:
            raise RpcError(-32601, "method not found: gc")
        if "--gc-fails" in ARGS:
            raise RpcError(-32000, "the toy cannot clean up today")
        wait_for = opt("--gc-waits-for", None)
        while wait_for and not os.path.exists(wait_for):
            time.sleep(0.05)
        keep = p["keep"]
        kept = lambda s: any(s == k or (k.endswith("*") and s.startswith(k[:-1])) for k in keep)
        doomed = [s for s in self.saves if not kept(s)]
        for s in doomed:
            del self.saves[s]
        return {"dropped": len(doomed), "bytes": len(doomed)}


def system():
    given = opt("--system", None)
    return {"system": json.loads(given)} if given else {}


RING = {"name": "ring", "description": "Ring the bell.", "inputSchema": {"type": "object", "properties": {}}, "effect": "read"}


class Bench:
    def describe(self, p):
        tools = [dict(TOOLS[0], description="The benchmark's own poke.")] if "--clash" in ARGS else []
        tools += [RING] if "--bench-tool" in ARGS else []
        return {"name": "toybench", "version": "0.1.0", "protocol": 1, "world": world(),
                "tasks": tasks(),
                "instructions": "Be kind to the box.", "tools": tools,
                "links": [json.loads(link) for link in opts("--link")],
                "gates": [json.loads(gate) for gate in opts("--gate")], "scored": "--scored" in ARGS}

    def seed(self, p):
        return {}

    def call_tool(self, p):
        if p["name"] == "ring":
            return {"result": {"rang": True}, "part": opt("--tool-part", "box"), "reads": [], "writes": []}
        return {"error": "the toy benchmark has no working tools", "part": "box", "reads": [], "writes": []}

    def score(self, p):
        n, need = p["docs"][world()[0]["part"]]["box/n"]["n"], int(p["task"][1:])
        checks = [{"name": "poked", "passed": n >= need, "detail": "n is %d" % n}]
        if "--safety" in ARGS:
            checks.append({"name": "not_overpoked", "passed": n <= need, "safety": True})
        return {"checks": checks}

    def gold_actions(self, p):
        return {"actions": [{"name": opt("--gold", "poke"), "args": json.loads(opt("--gold-args", "{}"))}]}


def tasks():
    once = {"id": "t1", "reason": "poke it", "ticket": "# Ticket\n\nPoke the box once.\n"}
    once["setup"] = [json.loads(call) for call in opts("--setup")]
    twice = {"id": "t2", "reason": "poke it twice", "ticket": "# Ticket\n\nPoke the box twice.\n"}
    return [once, twice] if "--two-tasks" in ARGS else [once]


def world():
    parts = [spec.split("=", 1) for spec in opts("--part")] or [["box", opt("--integration", "toy")]]
    configs = dict(spec.split("=", 1) for spec in opts("--config"))
    return [{"part": part, "integration": integration, "config": json.loads(configs.get(part, "{}"))}
            for part, integration in parts]


def log(request):
    path = opt("--log", None)
    if path:
        with open(path, "a") as f:
            f.write("%s %s\n" % (request.get("method"), (request.get("params") or {}).get("instance", "")))


def answer(handler, request):
    log(request)
    out = {"jsonrpc": "2.0", "id": request.get("id")}
    method = getattr(handler, request.get("method") or "", None)
    try:
        if method is None:
            raise RpcError(-32601, "method not found: %s" % request.get("method"))
        out["result"] = method(request.get("params") or {})
    except RpcError as e:
        out["error"] = {"code": e.code, "message": e.message}
    return out


def main():
    handler = Part() if ARGS[0] == "part" else Bench()
    for line in sys.stdin:
        if line.strip():
            print(json.dumps(answer(handler, json.loads(line))), flush=True)


main()
