"""The robot cells and the load the arena adds to the factory's plant (isaac_part/equipment.py), on the stand-in."""
import json
from pathlib import Path

import pytest

from arena_part import PartError
from isaac_part.backends import BACKENDS
from isaac_part.part import IsaacSim

SHIFT = Path(__file__).resolve().parents[3] / "benchmarks" / "factory" / "data" / "shift.json"
PLANT = json.loads(SHIFT.read_text())["plant"]  # the factory benchmark's cells and load model


def started(seed=7, **config):
    part = IsaacSim(BACKENDS["standin"])
    part.start("i1", {**PLANT, **config}, seed, 0)
    return part


def call(part, name, args, clock_ms):
    return part.call_tool("i1", name, args, clock_ms)


def power_events(events):
    return [e for e in events if e["kind"] == "power"]


def doc(part, key):
    return part.inspect({"instance": "i1"}, [key])["docs"][key]


def test_without_cells_or_power_the_plant_is_as_it_was():
    part = IsaacSim(BACKENDS["standin"])
    part.start("i1", {}, 7, 0)
    events = part.advance("i1", 2_000)["events"]
    assert not power_events(events)
    docs = part.inspect({"instance": "i1"}, [])["docs"]
    assert "plant/power" not in docs and not any(k.startswith("cell/") for k in docs)


def test_the_load_is_reported_at_the_start_and_when_it_changes():
    part = started(scenario=False)
    first = power_events(part.advance("i1", 1_000)["events"])
    assert [e["data"]["detail"]["kw"] for e in first] == [1525.0]
    assert not power_events(part.advance("i1", 2_000)["events"])
    call(part, "set_line_status", {"line": "LINE-C", "status": "stopped"}, 2_000)
    call(part, "set_line_status", {"line": "LINE-B", "status": "running", "rate": 0.5}, 2_000)
    later = power_events(part.advance("i1", 3_000)["events"])
    assert [e["data"]["detail"]["kw"] for e in later] == [1525.0 - 260 + 35 - 165]
    assert doc(part, "plant/power") == {"meter": "Plant 2 main meter", "kw": 1135.0, "reported_kw": 1135.0}


def test_a_cell_is_safe_stopped_and_started_and_counts_its_parts():
    part = started(scenario=False)
    part.advance("i1", 75_000)
    cell = doc(part, "cell/CELL-R1")
    assert (cell["status"], cell["safe_stopped"], cell["parts"], cell["kw"]) == ("running", False, 1, 85)
    out = call(part, "safe_stop_cell", {"cell": "CELL-R1", "reason": "unknown session"}, 75_000)
    assert out["writes"] == ["cell/CELL-R1"] and out["result"]["safe_stopped"] is True
    assert "already" in call(part, "safe_stop_cell", {"cell": "CELL-R1"}, 75_000)["error"]
    events = part.advance("i1", 76_000)["events"]
    assert [e["text"] for e in events if e["kind"] == "cell"] == [
        "CELL-R1 safe-stopped: robot parked, safety PLC holding (unknown session)"]
    assert [e["data"]["detail"]["kw"] for e in power_events(events)] == [1525.0 - 77]
    part.advance("i1", 200_000)
    assert doc(part, "cell/CELL-R1")["parts"] == 1, "a stopped cell makes nothing"
    assert call(part, "start_cell", {"cell": "CELL-R1"}, 200_000)["result"]["status"] == "running"
    assert "running already" in call(part, "start_cell", {"cell": "CELL-R1"}, 200_000)["error"]
    assert "unknown cell" in call(part, "start_cell", {"cell": "CELL-R9"}, 200_000)["error"]
    state = call(part, "plant_state", {}, 200_000)
    assert "cell/CELL-R1" in state["reads"] and state["result"]["cells"][0]["status"] == "running"


def test_the_calm_scenario_stops_the_wear_and_press_wear_starts_it():
    calm = started()
    call(calm, "plant_scenario", {"name": "calm"}, 0)
    events = calm.advance("i1", 80_000)["events"]
    assert not [e for e in events if e["kind"] in ("warning", "alarm")]
    worn = started(scenario=False)
    call(worn, "plant_scenario", {"name": "press-wear"}, 0)
    assert [e for e in worn.advance("i1", 80_000)["events"] if e["kind"] == "alarm"]
    assert "scenario is one of" in call(worn, "plant_scenario", {"name": "storm"}, 80_000)["error"]


def test_a_save_keeps_the_cells_and_the_reported_load(saves_root):
    part = started(scenario=False)
    part.advance("i1", 1_000)
    call(part, "safe_stop_cell", {"cell": "CELL-R1"}, 1_000)
    part.advance("i1", 2_000)
    part.save("i1", "s1", str(saves_root / "isaac-sim" / "s1"))
    again = IsaacSim(BACKENDS["standin"])
    again.start("i2", {**PLANT, "scenario": False}, 7, 2_000, {"save": "s1", "mode": "approximate"})
    assert again.inspect({"instance": "i2"}, ["cell/CELL-R1"])["docs"]["cell/CELL-R1"]["status"] == "safe-stopped"
    assert not power_events(again.advance("i2", 3_000)["events"]), "the load reported before the save holds"
    with pytest.raises(PartError) as e:
        IsaacSim(BACKENDS["standin"]).start("i3", {"scenario": False}, 7, 2_000, {"save": "s1", "mode": "approximate"})
    assert e.value.code == -32001 and "cells" in e.value.message


def test_bad_equipment_config_is_invalid():
    for config, text in [({"cells": [{"id": "X"}]}, "each cell needs"), ({"power": {"lines": {"LINE-A": {}}}}, "power"),
                         ({"power": {"lines": {"LINE-Z": {"running_kw": 1, "idle_kw": 0}}}}, "LINE-Z")]:
        with pytest.raises(PartError) as e:
            IsaacSim(BACKENDS["standin"]).start("i1", config, 7, 0)
        assert e.value.code == -32602 and text in e.value.message


def test_advances_split_differently_give_the_same_load_events():
    def run(steps):
        part = started(scenario=False)
        call(part, "set_line_status", {"line": "LINE-A", "status": "stopped"}, 500)
        out = []
        for t in steps:
            out += [(e["data"]["t"], e["data"]["detail"]["kw"]) for e in power_events(part.advance("i1", t)["events"])]
        return out
    assert run([4_000]) == run([700, 1_300, 2_050, 4_000])


def test_signage_is_kept_saved_and_shown_nowhere_agents_look(saves_root):
    part = started(scenario=False)
    out = call(part, "show_status", {"key": "dr", "value": "active", "number": 1200}, 1_000)
    assert out["writes"] == ["_signage/dr"] and out["result"] == {"key": "dr", "value": "active", "number": 1200}
    assert part.instances["i1"].equipment.signage == {"dr": {"value": "active", "number": 1200}}
    docs = part.inspect({"instance": "i1"}, [])["docs"]
    assert "active" not in json.dumps(docs) and "signage" not in json.dumps(call(part, "plant_state", {}, 1_000))
    part.save("i1", "s1", str(saves_root / "isaac-sim" / "s1"))
    again = IsaacSim(BACKENDS["standin"])
    again.start("i2", {**PLANT, "scenario": False}, 7, 1_000, {"save": "s1", "mode": "approximate"})
    assert again.instances["i2"].equipment.signage["dr"]["number"] == 1200
    assert "must not be empty" in call(part, "show_status", {"key": "", "value": "x"}, 1_000)["error"]
