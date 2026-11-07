"""Scheduled disruptions: armed by a scenario tool, each fires once, deterministically, before the
agent's call that crosses its count, even when that call is refused, and a save restores the count."""
import pytest

from arena_part import disruptions
from arena_part.jsonstate import Done, JsonStatePart, Tool
from arena_part.params import Param, Refusal


def _set_cap(config, state, do):
    state["cap"] = do["cap"]
    return ["box/cap"]


def _check_cap(config, do):
    if do["cap"] < 0:
        raise Refusal("cap must be 0 or more")


KINDS = {"cap": disruptions.Kind({"cap": "integer"}, _set_cap, _check_cap)}


def _scenario(config, state, args):
    fired = disruptions.arm(config, state, KINDS, list(Box.TOOLS), args["disruptions"])
    return Done({"armed": len(args["disruptions"])}, writes=["scenario/current", *fired.writes],
                events=fired.events)


def _take(config, state, args):
    if state["n"] >= state["cap"]:
        raise Refusal(f"full at {state['cap']}")
    state["n"] += 1
    return Done({"n": state["n"]}, writes=["box/n"])


def _peek(config, state, args):
    return Done({"n": state["n"], "cap": state["cap"]}, reads=["box/n"])


class Box(JsonStatePart):
    NAME = "box"
    TOOLS = {t.name: t for t in [
        Tool("scenario", "Arm disruptions.", "write", (Param("disruptions", "array"),), _scenario),
        Tool("take", "Take one.", "write", (), _take),
        Tool("peek", "Read.", "read", (), _peek)]}
    DISRUPTIONS = KINDS
    WORLD_TOOLS = frozenset({"scenario"})

    def describe_fields(self):
        return {"version": "0.1.0", "fingerprint": {"box": 1}}

    def new_state(self, config):
        return {"n": 0, "cap": 9}

    def docs(self, config, state):
        return {"box/n": {"n": state["n"], "cap": state["cap"]}, **disruptions.docs(state)}


def cap_after(n, tool=None, cap=0, did="full"):
    when = {"after_calls": n, **({"tool": tool} if tool else {})}
    return {"id": did, "when": when, "do": {"kind": "cap", "cap": cap}, "text": "The box is full."}


def armed(*given):
    box = Box()
    box.start("i1", {}, 0, 0)
    out = box.call_tool("i1", "scenario", {"disruptions": list(given)}, 0)
    assert "error" not in out, out
    return box, out


def call(box, name, instance="i1"):
    return box.call_tool(instance, name, {}, 0)


def test_it_fires_once_before_the_call_that_crosses_the_count():
    box, _ = armed(cap_after(2))
    first = call(box, "peek")
    assert first["result"]["cap"] == 9 and first["writes"] == ["disruption/full"]
    assert call(box, "take")["result"] == {"n": 1}
    third = call(box, "peek")
    assert third["result"]["cap"] == 0, "the crossing call already sees the change"
    assert third["events"] == [{"kind": "disruption", "text": "The box is full.",
                                "data": {"id": "full", "kind": "cap", "cap": 0}}]
    assert third["writes"] == ["disruption/full", "box/cap"]
    again = call(box, "peek")
    assert "events" not in again and again["writes"] == [], "it fires once; then reads write nothing"


def test_a_tool_counts_only_its_calls_and_a_refused_call_still_sets_it_off():
    box, _ = armed(cap_after(1, tool="take", cap=1))
    call(box, "peek")
    assert call(box, "take")["result"] == {"n": 1}
    refused = call(box, "take")
    assert refused["error"] == "full at 1"
    assert refused["events"][0]["data"]["id"] == "full", "the event comes even though the call refused"
    assert refused["writes"] == ["disruption/full", "box/cap"]
    assert box.inspect({"instance": "i1"}, ["disruption/full"])["docs"]["disruption/full"]["fired"]


def test_after_zero_calls_it_fires_as_it_is_armed_and_world_tools_count_for_nothing():
    _, out = armed(cap_after(0))
    assert out["events"][0]["kind"] == "disruption" and "box/cap" in out["writes"]
    box, _ = armed(cap_after(1))
    box.call_tool("i1", "scenario", {"disruptions": []}, 0)
    assert call(box, "peek")["result"]["cap"] == 9


def test_a_save_restores_the_count(tmp_path):
    box, _ = armed(cap_after(2))
    call(box, "peek")
    box.save("i1", "s1", str(tmp_path / "saves" / "box" / "s1"))
    box.start("i2", {}, 0, 0, {"save": "s1", "mode": "exact"})
    assert call(box, "peek", "i2")["result"]["cap"] == 9
    assert call(box, "peek", "i2")["result"]["cap"] == 0, "the restored count was 1"
    assert box.diff({"instance": "i1"}, {"save": "s1"}) == {"differing": []}


@pytest.mark.parametrize("bad, text", [
    ({"when": {"after_calls": -1}}, "after_calls"),
    ({"when": {"after_calls": 1, "tool": "nope"}}, "not a tool"),
    ({"do": {"kind": "boom"}}, "do.kind must be one of cap"),
    ({"do": {"kind": "cap", "cap": "x"}}, "do.cap must be of type integer"),
    ({"do": {"kind": "cap", "cap": -1}}, "cap must be 0 or more"),
    ({"text": None}, "text must be text"),
    ({"id": ""}, "needs an id"),
])
def test_a_bad_disruption_is_refused_and_arms_nothing(bad, text):
    box = Box()
    box.start("i1", {}, 0, 0)
    out = box.call_tool("i1", "scenario", {"disruptions": [{**cap_after(1), **bad}]}, 0)
    assert text in out["error"] and out["writes"] == []
    dup = box.call_tool("i1", "scenario", {"disruptions": [cap_after(1), cap_after(2)]}, 0)
    assert "same id" in dup["error"]
    assert box.inspect({"instance": "i1"}, [])["docs"] == {"box/n": {"n": 0, "cap": 9}}
