"""Toy arena-part/1 packages for arenad tests.

  toy.py part [--crash-on-call N [--crash-mark FILE]] [--save-fails-after N] [--bad-diff] [--log FILE]
      An integration with one counter per instance and tools `poke` (write, adds 1) and `peek`
      (read). It exits on its N-th call_tool (only once across restarts when FILE marks that it
      did); every `save` after the first N is an error; --bad-diff answers `diff` without
      `differing`; --log appends "<method> <instance>" per request to FILE. Saves are kept in
      memory. `inspect` refuses keys `bad/..` (-32602). No `connect` (method not found).
  toy.py bench --integration NAME [--clash] [--bench-tool [--tool-part PART]]
      A benchmark whose world is one part `box` of integration NAME, with one task `t1`. Tools:
      none; with --clash a `poke`; with --bench-tool a `ring` that reports it ran on PART
      (default `box`). Gold: one `poke`.
"""
import json
import os
import sys

ARGS = sys.argv[1:]


class RpcError(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code, self.message = code, message


def opt(name, default):
    return ARGS[ARGS.index(name) + 1] if name in ARGS else default


TOOLS = [
    {"name": "poke", "description": "Add one.", "inputSchema": {"type": "object", "properties": {}}, "effect": "write"},
    {"name": "peek", "description": "Read the counter.", "inputSchema": {"type": "object", "properties": {}}, "effect": "read"},
]


class Part:
    def __init__(self):
        self.crash_on = int(opt("--crash-on-call", "0"))
        self.saves_ok = int(opt("--save-fails-after", "-1"))
        self.calls = self.saved = 0
        self.counters, self.saves = {}, {}

    def describe(self, p):
        return {"name": "toy", "version": "0.1.0", "protocol": 1,
                "restore": {"exact": "save", "approximate": False}, "uses_clock": False,
                "fingerprint": {"toy": 1}, "tools": TOOLS}

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
        src = (p.get("from") or {}).get("save")
        self.counters[p["instance"]] = 0 if src is None else self.lookup(self.saves, src, "save")
        return {}

    def call_tool(self, p):
        self.calls += 1
        if self.crash_on and self.calls >= self.crash_on:
            self.crash()
        i = p["instance"]
        n = self.lookup(self.counters, i, "instance")
        if p["name"] == "poke":
            self.counters[i] = n + 1
            return {"result": {"n": n + 1}, "reads": [], "writes": ["box/n"]}
        if p["name"] == "peek":
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
        self.saved += 1
        if 0 <= self.saves_ok < self.saved:
            raise RpcError(-32000, "the toy can't save today")
        self.saves[p["save"]] = self.lookup(self.counters, p["instance"], "instance")
        return {"bytes": 1, "exact": True}

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
        self.saves.pop(p["save"], None)
        return {}

    def stop(self, p):
        self.counters.pop(p["instance"], None)
        return {}

    def usage(self, p):
        return {"bytes": len(self.saves)}


RING = {"name": "ring", "description": "Ring the bell.", "inputSchema": {"type": "object", "properties": {}}, "effect": "read"}


class Bench:
    def describe(self, p):
        tools = [dict(TOOLS[0], description="The benchmark's own poke.")] if "--clash" in ARGS else []
        tools += [RING] if "--bench-tool" in ARGS else []
        return {"name": "toybench", "version": "0.1.0", "protocol": 1,
                "world": [{"part": "box", "integration": opt("--integration", "toy"), "config": {}}],
                "tasks": [{"id": "t1", "reason": "poke it", "ticket": "# Ticket\n\nPoke the box once.\n"}],
                "instructions": "Be kind to the box.", "tools": tools}

    def seed(self, p):
        return {}

    def call_tool(self, p):
        if p["name"] == "ring":
            return {"result": {"rang": True}, "part": opt("--tool-part", "box"), "reads": [], "writes": []}
        return {"error": "the toy benchmark has no working tools", "part": "box", "reads": [], "writes": []}

    def gold_actions(self, p):
        return {"actions": [{"name": "poke", "args": {}}]}


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
