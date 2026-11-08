import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from conftest import CONFIG
from utility_part.part import Utility

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]
MIN = 60_000


def dr(after=0, target=1200, lead=10, minutes=30, tool=None, did="dr"):
    when = {"after_calls": after, **({"tool": tool} if tool else {})}
    do = {"kind": "demand_response", "event": "DR-1", "target_kw": target, "lead_min": lead, "minutes": minutes}
    return {"id": did, "when": when, "do": do, "text": "The utility calls a demand-response event"}


def started(*given, clock=0):
    part = Utility()
    part.start("i1", CONFIG, 0, clock)
    out = part.call_tool("i1", "utility_scenario", {"name": "base", "disruptions": list(given)}, clock)
    assert "error" not in out, out
    return part, out


def call(part, name, args, clock):
    return part.call_tool("i1", name, args, clock)


def doc(part, key):
    return part.inspect({"instance": "i1"}, [key])["docs"][key]


def test_the_meter_integrates_the_plants_load_on_the_clock():
    part, _ = started()
    assert part.describe()["uses_clock"] is True
    assert part.advance("i1", 30 * MIN) == {"changed": True, "events": []}
    call(part, "meter_load", {"kw": 1000, "cause": "Line C stopped"}, 30 * MIN)
    part.advance("i1", 60 * MIN)
    m = call(part, "utility_meter", {}, 60 * MIN)["result"]
    assert (m["kw"], m["peak_kw"], m["kwh_this_shift"]) == (1000.0, 1500.0, 1250.0)
    assert m["energy_cost_usd"] == 125.0 and m["tariff_period"] == "standard"
    assert m["at"] == "2026-03-10T07:00:00"


def test_how_the_advances_are_split_does_not_matter():
    def run(steps):
        part, _ = started(dr(lead=5, minutes=20))
        call(part, "meter_load", {"kw": 1300}, 7 * MIN)
        for t in steps:
            part.advance("i1", t)
        return part.inspect({"instance": "i1"}, [])["docs"]
    assert run([40 * MIN]) == run([MIN, 5 * MIN, 7 * MIN, 13 * MIN, 25 * MIN, 39 * MIN, 40 * MIN])


def test_a_demand_response_window_opens_and_closes_and_is_measured():
    part, out = started(dr(lead=10, minutes=30))
    assert out["events"][0]["kind"] == "disruption" and "dr/DR-1" in out["writes"]
    ev = call(part, "demand_response_events", {}, 0)["result"]["events"][0]
    assert (ev["status"], ev["starts_at"], ev["ends_at"]) == ("notified", "2026-03-10T06:10:00", "2026-03-10T06:40:00")
    assert part.advance("i1", 10 * MIN)["events"][0]["kind"] == "demand_response_active"
    call(part, "meter_load", {"kw": 1100}, 20 * MIN)  # 10 min at 1500 over target, then under it
    assert call(part, "utility_meter", {}, 20 * MIN)["result"]["tariff_period"] == "peak"
    ended = part.advance("i1", 50 * MIN)["events"]
    assert [e["kind"] for e in ended] == ["demand_response_ended"]
    ev = doc(part, "dr/DR-1")
    assert (ev["status"], ev["minutes_in_window"], ev["minutes_over_target"]) == ("ended", 30.0, 10.0)
    assert (ev["avg_kw_in_window"], ev["max_kw_in_window"]) == (1233.3, 1500.0)
    kwh = doc(part, "meter/site")["kwh_this_shift"]
    assert kwh == pytest.approx(1500 * 20 / 60 + 1100 * 30 / 60)


def test_a_window_event_found_in_a_call_waits_for_the_next_advance():
    part, _ = started(dr(lead=1, minutes=1))
    call(part, "utility_meter", {}, 5 * MIN)
    kinds = [e["kind"] for e in part.advance("i1", 5 * MIN)["events"]]
    assert kinds == ["demand_response_active", "demand_response_ended"]


def test_the_notice_comes_only_after_the_counted_calls_and_world_calls_count_for_nothing():
    part, out = started(dr(after=2, tool="utility_meter"))
    assert "events" not in out
    call(part, "meter_load", {"kw": 900}, 0)
    for _ in range(2):
        assert "events" not in call(part, "utility_meter", {}, MIN)
    fired = call(part, "demand_response_events", {}, MIN)
    assert "events" not in fired and fired["result"]["events"] == []
    third = call(part, "utility_meter", {}, 2 * MIN)
    assert third["events"][0]["data"]["event"] == "DR-1"
    assert doc(part, "dr/DR-1")["notice_at"] == "2026-03-10T06:02:00"


def test_a_response_is_recorded_and_bad_ones_are_refused():
    part, _ = started(dr())
    ok = call(part, "respond_to_event", {"event": "DR-1", "committed_kw": 1150, "plan": "Stop Line C"}, MIN)
    assert ok["writes"] == ["dr/DR-1"] and doc(part, "dr/DR-1")["acknowledged"] is True
    assert "unknown" in call(part, "respond_to_event", {"event": "X", "committed_kw": 1, "plan": "p"}, MIN)["error"]
    part.advance("i1", 60 * MIN)
    late = call(part, "respond_to_event", {"event": "DR-1", "committed_kw": 1, "plan": "p"}, 60 * MIN)
    assert "has ended" in late["error"] and late["writes"] == []
    assert "kw must be" in call(part, "meter_load", {"kw": -1}, 60 * MIN)["error"]


def test_a_save_restores_the_meter_the_events_and_the_schedule(tmp_path):
    part, _ = started(dr(after=1, tool="utility_meter", lead=0, minutes=60))
    call(part, "utility_meter", {}, 3 * MIN)
    part.save("i1", "s1", str(tmp_path / "saves" / "utility" / "s1"))
    again = Utility()
    again.start("i2", CONFIG, 0, 3 * MIN, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    fired = again.call_tool("i2", "utility_meter", {}, 4 * MIN)
    assert fired["events"][0]["kind"] == "disruption"
    assert again.inspect({"instance": "i2"}, ["dr/DR-1"])["docs"]["dr/DR-1"]["status"] == "active"
    for bad in [{"initial_kw": -5}, {"tariff": {"standard_per_kwh": 1}}, {"plant_start": "soon"}]:
        with pytest.raises(PartError):
            Utility().start("i3", bad, 0, 0)


def test_a_bad_disruption_is_refused():
    part = Utility()
    part.start("i1", CONFIG, 0, 0)
    bad = dr()
    bad["do"]["minutes"] = 0
    out = call(part, "utility_scenario", {"name": "base", "disruptions": [bad]}, 0)
    assert "minutes 1 or more" in out["error"]
    assert "unknown scenario" in call(part, "utility_scenario", {"name": "nope"}, 0)["error"]


def test_conformance(tmp_path):
    """`arena-part-check` (describe, start, the examples, the clock, save and restore, diff, drop_save)."""
    bin_dir = Path(os.environ.get("ARENA_BIN_DIR", ARENA / "core" / "target" / "debug"))
    check = bin_dir / "arena-part-check"
    if not check.exists() or shutil.which("uv") is None:
        pytest.skip(f"no {check} (cargo build -p parts --bin arena-part-check) or no uv")
    config = tmp_path / "config.json"
    config.write_text(json.dumps(CONFIG))
    env = {**os.environ, "ARENA_BIN_DIR": str(bin_dir)}
    run = subprocess.run([str(check), str(PACKAGE), "--config", str(config)], capture_output=True, text=True,
                         env=env, timeout=300)
    assert run.returncode == 0, run.stdout + run.stderr
