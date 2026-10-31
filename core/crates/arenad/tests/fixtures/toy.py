"""Toy arena-part/1 packages for arenad tests.

  toy.py part [--crash-on-call N] [--save-fails-after N]
      An integration with one counter per instance and tools `poke` (write, adds 1) and `peek`
      (read). It exits on its N-th call_tool; every `save` after the first N is an error. Saves
      are kept in memory. No `connect` (method not found).
  toy.py bench --integration NAME [--clash]
      A benchmark whose world is one part `box` of integration NAME, with one task `t1` and no
      tools (with --clash, a tool `poke` too). Gold: one `poke`.
"""
import json
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
            sys.stderr.write("toy: crashing on call %d\n" % self.calls)
            sys.stderr.flush()
            sys.exit(7)
        i = p["instance"]
        n = self.lookup(self.counters, i, "instance")
        if p["name"] == "poke":
            self.counters[i] = n + 1
            return {"result": {"n": n + 1}, "reads": [], "writes": ["box/n"]}
        if p["name"] == "peek":
            return {"result": {"n": n}, "reads": ["box/n"], "writes": []}
        return {"error": "unknown tool " + p["name"], "reads": [], "writes": []}

    def save(self, p):
        self.saved += 1
        if 0 <= self.saves_ok < self.saved:
            raise RpcError(-32000, "the toy can't save today")
        self.saves[p["save"]] = self.lookup(self.counters, p["instance"], "instance")
        return {"bytes": 1, "exact": True}

    def inspect(self, p):
        v = self.value(p["target"])
        return {"docs": {k: ({"n": v} if k == "box/n" else None) for k in (p.get("keys") or ["box/n"])}}

    def diff(self, p):
        return {"differing": [] if self.value(p["a"]) == self.value(p["b"]) else ["box/n"]}

    def drop_save(self, p):
        self.saves.pop(p["save"], None)
        return {}

    def stop(self, p):
        self.counters.pop(p["instance"], None)
        return {}

    def usage(self, p):
        return {"bytes": len(self.saves)}


class Bench:
    def describe(self, p):
        tools = [dict(TOOLS[0], description="The benchmark's own poke.")] if "--clash" in ARGS else []
        return {"name": "toybench", "version": "0.1.0", "protocol": 1,
                "world": [{"part": "box", "integration": opt("--integration", "toy"), "config": {}}],
                "tasks": [{"id": "t1", "reason": "poke it", "ticket": "# Ticket\n\nPoke the box once.\n"}],
                "instructions": "Be kind to the box.", "tools": tools}

    def seed(self, p):
        return {}

    def call_tool(self, p):
        return {"error": "the toy benchmark has no working tools", "part": "box", "reads": [], "writes": []}

    def gold_actions(self, p):
        return {"actions": [{"name": "poke", "args": {}}]}


def answer(handler, request):
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
