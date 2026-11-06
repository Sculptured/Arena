"""A JSON-state part: refusals change nothing, saves restore exactly, inspect and diff read the docs."""
import pytest

from arena_part import PartError
from arena_part.jsonstate import Done, JsonStatePart, Tool, event
from arena_part.params import Param, Refusal


def _add(config, state, args):
    state["n"] += args["n"]
    if state["n"] > config["max"]:
        raise Refusal(f"over {config['max']}")
    return Done({"n": state["n"]}, writes=["box/n"], events=[event("added", "added", {"n": state["n"]})])


class Box(JsonStatePart):
    NAME = "box"
    TOOLS = {"add": Tool("add", "Add n.", "write", (Param("n", "integer"),), _add)}

    def describe_fields(self):
        return {"version": "0.1.0", "fingerprint": {"box": 1}}

    def new_state(self, config):
        return {"n": 0}

    def docs(self, config, state):
        return {"box/n": {"n": state["n"], "max": config["max"]}}


def test_a_refusal_changes_nothing_and_a_call_gives_its_events():
    box = Box()
    box.start("i1", {"max": 5}, 0, 0)
    out = box.call_tool("i1", "add", {"n": 4}, 0)
    assert out == {"result": {"n": 4}, "reads": [], "writes": ["box/n"],
                   "events": [{"kind": "added", "text": "added", "data": {"n": 4}}]}
    assert box.call_tool("i1", "add", {"n": 4}, 0) == {"error": "over 5", "reads": [], "writes": []}
    assert box.call_tool("i1", "add", {}, 0)["error"] == "add: missing n"
    assert box.inspect({"instance": "i1"}, [])["docs"] == {"box/n": {"n": 4, "max": 5}}
    with pytest.raises(PartError) as e:
        box.call_tool("i1", "nope", {}, 0)
    assert e.value.code == -32602


def test_a_save_restores_exactly_and_diffs(tmp_path):
    box = Box()
    box.start("i1", {"max": 9}, 0, 0)
    box.call_tool("i1", "add", {"n": 2}, 0)
    assert box.save("i1", "s1", str(tmp_path / "saves" / "box" / "s1"))["exact"] is True
    box.call_tool("i1", "add", {"n": 1}, 0)
    box.start("i2", {"max": 9}, 0, 0, {"save": "s1", "mode": "exact"})
    assert box.inspect({"save": "s1"}, ["box/n"])["docs"]["box/n"]["n"] == 2
    assert box.diff({"instance": "i1"}, {"save": "s1"}) == {"differing": ["box/n"]}
    assert box.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    box.drop_save("s1")
    with pytest.raises(PartError) as e:
        box.inspect({"save": "s1"}, [])
    assert e.value.code == -32002
