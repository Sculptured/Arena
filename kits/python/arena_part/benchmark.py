"""Benchmarks in Python: the calls of docs/protocol.md, "Benchmark calls", over the same line protocol.

Subclass `Benchmark`, override what you need (`describe` always; `score` with `scored: true`; `view` with
`view`), and end the program with `serve_benchmark(MyBenchmark())`.
"""
from . import errors
from .serve import _need, serve


class Benchmark:
    """A benchmark: names its world and tasks, seeds fresh instances, and scores or draws the world from
    its parts' docs. Methods take and return plain JSON values."""

    def describe(self):
        raise NotImplementedError("describe")

    def seed(self, connections):
        return None  # a world of simulated parts starts from their configs

    def call_tool(self, name, args, connections):
        return {"error": f"this benchmark has no tool {name}", "part": "", "reads": [], "writes": []}

    def gold_actions(self, task):
        return {"actions": []}

    def score(self, task, docs):
        raise errors.method_not_found("score")

    def view(self, connections, docs):
        raise errors.method_not_found("view")


def _seed(bench, params):
    bench.seed(*_need(params, "connections"))
    return {}


_METHODS = {
    "describe": lambda b, p: b.describe(),
    "seed": _seed,
    "call_tool": lambda b, p: b.call_tool(*_need(p, "name", "args", "connections")),
    "gold_actions": lambda b, p: b.gold_actions(*_need(p, "task")),
    "score": lambda b, p: b.score(*_need(p, "task", "docs")),
    "view": lambda b, p: b.view(*_need(p, "connections", "docs")),
}


def dispatch(bench, method, params):
    handler = _METHODS.get(method)
    if handler is None:
        raise errors.method_not_found(method)
    return handler(bench, params)


def serve_benchmark(bench, stdin=None, stdout=None):
    """Serve a benchmark until the input closes."""
    serve(bench, stdin, stdout, dispatch_fn=dispatch)
