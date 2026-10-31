import io
import json

import pytest

from arena_part import serve


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root


class Client:
    """Drives `serve` with in-memory streams: send lines, run, read replies."""

    def __init__(self, part):
        self.part = part
        self.lines = []
        self.next = 1

    def req(self, method, params=None, **extra):
        msg = {"jsonrpc": "2.0", "id": self.next, "method": method, "params": params or {}}
        msg.update(extra)
        self.next += 1
        self.lines.append(json.dumps(msg))
        return msg["id"]

    def raw(self, line):
        self.lines.append(line)

    def run(self):
        out = io.StringIO()
        serve(self.part, io.StringIO("\n".join(self.lines) + "\n"), out)
        self.lines = []
        return [json.loads(l) for l in out.getvalue().splitlines()]

    def call(self, method, params=None):
        self.req(method, params)
        return self.run()[0]
