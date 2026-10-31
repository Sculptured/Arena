import json

import pytest

from arena_part import Part, PartError, FixedStep, serve
import counter
from conftest import Client


def client(flavour="exact"):
    return Client(counter.Counter(counter.Flavour.parse(flavour)))


def save_dir(root, save):
    return str(root / "counter-py" / save)


def start(c, instance, **extra):
    r = c.call("start", {"instance": instance, "config": {}, "seed": 1, "clock_ms": 0, **extra})
    assert "error" not in r, r


def get(c, instance):
    r = c.call("call_tool", {"instance": instance, "name": "get", "args": {}, "clock_ms": 0})
    return r["result"]["result"]["value"]


def add(c, instance, n):
    r = c.call("call_tool", {"instance": instance, "name": "add", "args": {"n": n}, "clock_ms": 0})
    assert "error" not in r, r


def test_describe_reports_protocol_1():
    r = client().call("describe")
    assert r["result"]["protocol"] == 1
    assert "inputSchema" in r["result"]["tools"][0]
    assert r["result"]["fingerprint"] == {"example": "counter"}


def test_unknown_method_is_32601():
    assert client().call("nope")["error"]["code"] == -32601


def test_bad_instance_id_is_32602():
    r = client().call("start", {"instance": "Bad Id", "config": {}, "seed": 1, "clock_ms": 0})
    assert r["error"]["code"] == -32602
    assert r["error"]["message"] == "instance and save ids must match [a-z0-9_]{1,63}"


@pytest.mark.parametrize(
    "method,params",
    [
        ("inspect", {"target": {"save": "Bad"}, "keys": []}),
        ("diff", {"a": {"instance": "i1"}, "b": {"save": "no way"}}),
        ("start", {"instance": "i1", "config": {}, "seed": 1, "clock_ms": 0,
                   "from": {"save": "X", "mode": "exact"}}),
        ("gc", {"keep": ["ok", "NOT ok"]}),
        ("stop", {"instance": 5}),
    ],
)
def test_nested_ids_are_validated(method, params):
    assert client().call(method, params)["error"]["code"] == -32602


def test_missing_params_is_32602():
    assert client().call("start", {"instance": "i1"})["error"]["code"] == -32602


def test_parse_error_and_invalid_request():
    c = client()
    c.raw("{not json")
    c.raw(json.dumps({"jsonrpc": "2.0", "id": 7}))
    out = c.run()
    assert out[0]["error"]["code"] == -32700 and out[0]["id"] is None
    assert out[1]["error"]["code"] == -32600 and out[1]["id"] == 7


def test_notification_gets_no_reply():
    c = client()
    c.raw(json.dumps({"jsonrpc": "2.0", "method": "usage", "params": {}}))
    c.req("usage")
    out = c.run()
    assert len(out) == 1 and out[0]["id"] == 1


def test_save_then_start_from_save_restores(saves_root):
    c = client()
    start(c, "i1")
    add(c, "i1", 5)
    r = c.call("save", {"instance": "i1", "save": "s1", "dir": save_dir(saves_root, "s1")})
    assert r["result"]["exact"] is True, r
    assert r["result"]["bytes"] == 20, r
    add(c, "i1", 2)
    start(c, "i2", **{"from": {"save": "s1", "mode": "exact"}})
    assert get(c, "i2") == 5
    assert get(c, "i1") == 7


def test_fresh_process_starts_from_other_processs_save(saves_root):
    a = client()
    start(a, "i1")
    add(a, "i1", 9)
    a.call("save", {"instance": "i1", "save": "s_x", "dir": save_dir(saves_root, "s_x")})
    b = client()
    start(b, "i1", **{"from": {"save": "s_x", "mode": "exact"}})
    assert get(b, "i1") == 9
    r = b.call("inspect", {"target": {"save": "s_x"}, "keys": []})
    assert r["result"]["docs"]["counter/value"] == 9


def test_unknown_save_is_32002():
    c = client()
    r = c.call("start", {"instance": "i1", "config": {}, "seed": 1, "clock_ms": 0,
                         "from": {"save": "nope", "mode": "exact"}})
    assert r["error"]["code"] == -32002, r
    r = c.call("inspect", {"target": {"save": "nope"}, "keys": []})
    assert r["error"]["code"] == -32002


def test_flavours_describe_their_restore():
    for f, exact, approx in [
        ("exact", "save", False),
        ("replay_only", "replay", False),
        ("approx_and_replay", "replay", True),
        ("broken_restore", "save", False),
        ("nondeterministic", "replay", False),
    ]:
        r = client(f).call("describe")
        assert r["result"]["restore"] == {"exact": exact, "approximate": approx}, f


def test_replay_only_save_is_32001(saves_root):
    c = client("replay_only")
    start(c, "i1")
    r = c.call("save", {"instance": "i1", "save": "s_r", "dir": save_dir(saves_root, "s_r")})
    assert r["error"]["code"] == -32001, r


def test_approx_save_reports_inexact(saves_root):
    c = client("approx_and_replay")
    start(c, "i1")
    r = c.call("save", {"instance": "i1", "save": "s_a", "dir": save_dir(saves_root, "s_a")})
    assert r["result"]["exact"] is False, r


def test_broken_restore_restores_zero(saves_root):
    c = client("broken_restore")
    start(c, "i1")
    add(c, "i1", 5)
    c.call("save", {"instance": "i1", "save": "s_b", "dir": save_dir(saves_root, "s_b")})
    start(c, "i2", **{"from": {"save": "s_b", "mode": "exact"}})
    assert get(c, "i2") == 0


def test_nondeterministic_add_varies():
    c = client("nondeterministic")
    start(c, "i1")
    seen = set()
    for _ in range(8):
        add(c, "i1", 0)
        seen.add(get(c, "i1"))
    assert len(seen) > 1


def test_advance_is_split_independent():
    one, many = client(), client()
    start(one, "i1")
    start(many, "i1")
    one.call("advance", {"instance": "i1", "clock_ms": 2500})
    for t in [500, 1000, 1500, 2000, 2500]:
        many.call("advance", {"instance": "i1", "clock_ms": t})
    assert get(one, "i1") == 2
    assert get(many, "i1") == 2


class Odd(Part):
    def call_tool(self, instance, name, args, clock_ms):
        raise RuntimeError("boom")

    def usage(self):
        return {"bytes": 0}


def test_raising_handler_still_answers():
    c = Client(Odd())
    r = c.call("call_tool", {"instance": "i1", "name": "x", "args": {}, "clock_ms": 0})
    assert r["error"]["code"] == -32603
    assert r["error"]["message"] == "internal error: boom"
    assert "result" in c.call("usage")


def test_back_to_back_requests_answered_in_order():
    c = client()
    ids = [c.req("describe"), c.req("usage"), c.req("nope")]
    out = c.run()
    assert [r["id"] for r in out] == ids
    assert "result" in out[0] and "result" in out[1] and out[2]["error"]["code"] == -32601


def test_fixed_step_keeps_remainder():
    steps = []
    f = FixedStep(100)
    f.advance_to(250, steps.append)
    f.advance_to(299, steps.append)
    f.advance_to(300, steps.append)
    assert steps == [100, 100, 100]
