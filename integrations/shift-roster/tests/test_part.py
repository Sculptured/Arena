import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from conftest import config, front
from shift_roster_part.part import ShiftRoster

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]


def callout(after=0, tool=None):
    when = {"after_calls": after, **({"tool": tool} if tool else {})}
    return {"id": "callout", "when": when, "do": {"kind": "callout", **front("callout")},
            "text": "Mei Tanaka and Priya Shah call out"}


def started(*given):
    part = ShiftRoster()
    part.start("i1", config(), 0, 0)
    out = call(part, "roster_scenario", {"name": "base", "disruptions": list(given)})
    assert "error" not in out, out
    return part, out


def call(part, name, args):
    return part.call_tool("i1", name, args, 0)


def station(part, sid):
    key = f"station/{sid}"
    return part.inspect({"instance": "i1"}, [key])["docs"][key]


def test_every_station_is_staffed_at_the_start():
    part, _ = started()
    out = call(part, "shift_roster", {})["result"]
    assert [(s["id"], s["operator_name"], s["staffed"]) for s in out["stations"]] == [
        ("LINE-A", "Jordan Marsh", True), ("LINE-B", "Priya Shah", True), ("LINE-C", "Tom Ellery", True),
        ("CELL-R1", "Mei Tanaka", True)]
    assert out["overtime"] == {"called_in": 0, "hours": 0, "cost_usd": 0}


def test_a_callout_leaves_stations_unstaffed_and_a_call_in_covers_the_cell():
    part, out = started(callout())
    assert out["events"][0]["kind"] == "disruption"
    assert {"station/CELL-R1", "station/LINE-B", "operator/OP-14", "operator/OP-12"} <= set(out["writes"])
    cell = station(part, "CELL-R1")
    assert (cell["staffed"], cell["why_not"]) == (False, "Mei Tanaka is not on shift (called out)")
    assert "not on shift" in call(part, "assign_operator", {"operator": "Mei Tanaka", "station": "CELL-R1"})["error"]
    assert "call them in first" in call(part, "assign_operator", {"operator": "OP-16", "station": "CELL-R1"})["error"]
    kim = call(part, "assign_operator", {"operator": "Kim Adair", "station": "CELL-R1"})["result"]
    assert kim["staffed"] is False and "not certified for robot-cell" in kim["warning"]
    assert call(part, "assign_operator", {"operator": "Kim Adair", "station": "LINE-B"})["result"]["staffed"]
    assert station(part, "CELL-R1")["operator"] is None
    called = call(part, "call_in_operator", {"operator": "Luis Ortega"})["result"]
    assert called["overtime"] == {"called_in": 1, "hours": 6, "cost_usd": 387.0}
    assert call(part, "assign_operator", {"operator": "Luis Ortega", "station": "CELL-R1"})["result"]["staffed"]
    assert "not on call" in call(part, "call_in_operator", {"operator": "Mei Tanaka"})["error"]
    assert "on shift already" in call(part, "call_in_operator", {"operator": "OP-16"})["error"]


def test_a_station_taken_over_leaves_its_operator_floating():
    part, _ = started()
    out = call(part, "assign_operator", {"operator": "OP-15", "station": "LINE-C"})
    assert set(out["writes"]) == {"operator/OP-15", "station/LINE-C", "operator/OP-13"}
    tom = part.inspect({"instance": "i1"}, ["operator/OP-13"])["docs"]["operator/OP-13"]
    assert tom["station"] is None and tom["on_shift"] is True
    assert "unknown station" in call(part, "assign_operator", {"operator": "OP-15", "station": "X"})["error"]
    assert "unknown operator" in call(part, "assign_operator", {"operator": "Nobody", "station": "LINE-C"})["error"]


def test_a_save_restores_everything(tmp_path):
    part, _ = started(callout(after=1, tool="shift_roster"))
    call(part, "shift_roster", {})
    call(part, "shift_roster", {})
    part.save("i1", "s1", str(tmp_path / "saves" / "shift-roster" / "s1"))
    again = ShiftRoster()
    again.start("i2", config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    assert again.inspect({"instance": "i2"}, ["station/LINE-B"])["docs"]["station/LINE-B"]["staffed"] is False
    with pytest.raises(PartError):
        ShiftRoster().start("i3", {"stations": [], "operators": []}, 0, 0)
    bad = callout()
    bad["do"]["operators"] = ["Nobody"]
    assert "unknown operator" in call(part, "roster_scenario", {"name": "base", "disruptions": [bad]})["error"]


def test_conformance(tmp_path):
    """`arena-part-check` (describe, start, the examples, save and restore, diff, drop_save)."""
    bin_dir = Path(os.environ.get("ARENA_BIN_DIR", ARENA / "core" / "target" / "debug"))
    check = bin_dir / "arena-part-check"
    if not check.exists() or shutil.which("uv") is None:
        pytest.skip(f"no {check} (cargo build -p parts --bin arena-part-check) or no uv")
    path = tmp_path / "config.json"
    path.write_text(json.dumps(config()))
    env = {**os.environ, "ARENA_BIN_DIR": str(bin_dir)}
    run = subprocess.run([str(check), str(PACKAGE), "--config", str(path)], capture_output=True, text=True,
                         env=env, timeout=300)
    assert run.returncode == 0, run.stdout + run.stderr
