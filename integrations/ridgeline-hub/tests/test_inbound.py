"""The raw-material trucks the arena adds to the hub (ridgeline_hub_part/inbound.py) and their scheduled delay."""
import json
from pathlib import Path

import pytest

from arena_part import PartError
from ridgeline_hub_part.part import RidgelineHub

SHIFT = json.loads((Path(__file__).resolve().parents[3] / "benchmarks" / "factory" / "data" / "shift.json").read_text())
INBOUND = SHIFT["inbound"]  # the factory benchmark's coil truck
HOUR = 3_600_000


def delay(after=0, tool=None, hours=6):
    when = {"after_calls": after, **({"tool": tool} if tool else {})}
    return {"id": "truck-late", "when": when, "text": "The coil truck is late",
            "do": {"kind": "delivery_delay", "delivery": "INB-0310-1", "extra_hours": hours}}


def started(*given):
    part = RidgelineHub()
    part.start("i1", {"inbound": INBOUND}, 1, 0)
    out = part.call_tool("i1", "hub_scenario", {"name": "base", "disruptions": list(given)}, 0)
    assert "error" not in out, out
    return part, out


def call(part, name, args, clock_ms):
    return part.call_tool("i1", name, args, clock_ms)


def truck(part, clock_ms):
    return call(part, "inbound_deliveries", {}, clock_ms)["result"]["deliveries"][0]


def test_the_truck_arrives_on_the_plant_clock_before_the_line_runs_out():
    part, _ = started()
    t = truck(part, HOUR)
    assert (t["eta"], t["status"], t["line_runs_out"], t["line_short_hours"]) == (
        "2026-03-10T08:30:00", "in transit", "2026-03-10T09:00:00", 0.0)
    assert truck(part, 3 * HOUR)["status"] == "arrived"
    doc = part.inspect({"instance": "i1"}, ["inbound/INB-0310-1"])["docs"]["inbound/INB-0310-1"]
    assert doc["status"] == "arrived" and doc["carrier"] == "Lakeshore Freight"


def test_a_delay_fires_before_the_counted_call_even_a_market_one():
    part, out = started(delay(after=1))
    assert "events" not in out
    call(part, "customer_replies", {}, 0)
    fired = call(part, "get_quotes", {"rfq": "RFQ-404"}, 0)
    assert "unknown rfq" in fired["error"] and fired["events"][0]["kind"] == "disruption"
    assert fired["writes"] == ["_disruption/truck-late", "inbound/INB-0310-1"]
    t = truck(part, HOUR)
    assert (t["eta"], t["delayed_hours"], t["line_short_hours"]) == ("2026-03-10T14:30:00", 6, 5.5)


def test_a_delay_counts_only_its_tool_and_survives_a_save(saves_root):
    part, _ = started(delay(after=1, tool="inbound_deliveries"))
    call(part, "customer_replies", {}, 0)
    assert truck(part, 0)["delayed_hours"] == 0
    part.save("i1", "s1", str(saves_root / "ridgeline-hub" / "s1"))
    again = RidgelineHub()
    again.start("i2", {"inbound": INBOUND}, 1, 0, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    out = again.call_tool("i2", "inbound_deliveries", {}, 0)
    assert out["events"][0]["data"]["extra_hours"] == 6 and out["result"]["deliveries"][0]["delayed_hours"] == 6


def test_bad_deliveries_and_delays_are_refused():
    part = RidgelineHub()
    part.start("i1", {"inbound": INBOUND}, 1, 0)
    bad = delay()
    bad["do"]["delivery"] = "INB-9"
    assert "no inbound delivery" in call(part, "hub_scenario", {"name": "base", "disruptions": [bad]}, 0)["error"]
    assert "unknown scenario" in call(part, "hub_scenario", {"name": "storm"}, 0)["error"]
    with pytest.raises(PartError) as e:
        RidgelineHub().start("i2", {"inbound": [{"id": "X"}]}, 1, 0)
    assert e.value.code == -32602
