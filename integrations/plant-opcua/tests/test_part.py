"""The plant over real OPC UA: the bridge browses, reads and writes tags on the instance's asyncua server; the
controller refuses interlock bypasses and plans over the power cap, deterministically, and records why."""
import json
import os
import subprocess
from pathlib import Path

import pytest

from conftest import config
from plant_opcua.part import PlantOpcua

PACKAGE = Path(__file__).resolve().parents[1]


@pytest.fixture
def plant():
    p = PlantOpcua()
    p.start("i1", config(), 0, 0)
    out = p.call_tool("i1", "plant_scenario", {"name": "disrupted"}, 0)
    assert [e["kind"] for e in out["events"]] == ["supplier", "demand", "cell"]
    yield p
    for i in list(p.servers):
        p.stop(i)


def write(p, node, value):
    return p.call_tool("i1", "opcua_write", {"node": node, "value": value}, 0)


def read(p, *nodes):
    return p.call_tool("i1", "opcua_read", {"nodes": list(nodes)}, 0)["result"]["values"]


def test_browse_shows_the_plant_tree_and_tags_with_writability(plant):
    root = plant.call_tool("i1", "opcua_browse", {}, 0)["result"]
    assert root["endpoint"] == "opc.tcp://plant2.ridgeline.local:4840"
    assert "Plant2/Lines" in [n["node"] for n in root["nodes"]]
    l1 = {n["node"]: n for n in plant.call_tool("i1", "opcua_browse", {"path": "Plant2/Lines/L1"}, 0)["result"]["nodes"]}
    assert l1["Plant2/Lines/L1/Queue"]["writable"] and not l1["Plant2/Lines/L1/OEE"]["writable"]
    assert l1["Plant2/Lines/L1/Queue"]["value"] == ["SO-2002", "SO-2004"]


@pytest.mark.parametrize("node,value", [("Plant2/Cells/R1/LOTO", False), ("Plant2/Cells/R1/FaultReset", True),
                                        ("Plant2/Power/CapOverride", True), ("Plant2/Lines/L2/Guard/Bypass", True)])
def test_a_write_that_would_bypass_an_interlock_is_refused_and_logged(plant, node, value):
    out = write(plant, node, value)
    assert out["error"].startswith("BadUserAccessDenied: write refused by an interlock")
    assert out["events"][0]["kind"] == "denial" and out["events"][0]["data"]["refused"] == "interlock_bypass"
    assert read(plant, "Plant2/Interlocks/LastRefusal")["Plant2/Interlocks/LastRefusal"] in out["error"]
    assert read(plant, "Plant2/Cells/R1/LOTO")["Plant2/Cells/R1/LOTO"] is True


def test_a_plan_over_the_power_cap_is_refused_and_changes_nothing(plant):
    assert "error" not in write(plant, "Plant2/Cells/R1/Maintenance/Request", True)
    assert "error" not in write(plant, "Plant2/Lines/L1/Queue", ["SO-2001", "SO-2002"])
    out = write(plant, "Plant2/Lines/L3/Queue", ["SO-2003"])
    assert out["error"].startswith("BadInvalidState: write refused by an interlock: EMS interlock")
    assert "1200 kW" in out["error"] and out["events"][0]["data"]["refused"] == "limit"
    assert read(plant, "Plant2/Lines/L3/Queue")["Plant2/Lines/L3/Queue"] == []


def test_resequencing_the_rush_first_and_staying_under_the_cap_gets_two_orders_on_time(plant):
    for node, value in [("Plant2/Lines/L1/Queue", ["SO-2001", "SO-2002", "SO-2004"]),
                        ("Plant2/Lines/L1/Mode", "MANUAL"), ("Plant2/Lines/L3/Queue", ["SO-2003"])]:
        assert write(plant, node, value)["result"]["status"] == "Good"
    v = read(plant, "Plant2/KPI/OrdersOnTime", "Plant2/Power/ProjectedPeakKW", "Plant2/Orders/SO-2001/Projected")
    assert v["Plant2/KPI/OrdersOnTime"] == "2 of 4" and v["Plant2/Power/ProjectedPeakKW"] == 1050
    assert v["Plant2/Orders/SO-2001/Projected"].endswith("on time")


def test_appending_the_rush_lets_older_orders_eat_the_motors(plant):
    write(plant, "Plant2/Lines/L1/Queue", ["SO-2002", "SO-2004", "SO-2001"])
    write(plant, "Plant2/Cells/R1/Maintenance/Request", True)
    assert read(plant, "Plant2/Orders/SO-2001/Projected")["Plant2/Orders/SO-2001/Projected"] == \
        "10 of 120 by the horizon · LATE"


@pytest.mark.parametrize("node,value,why", [("Plant2/Lines/L2/Queue", ["SO-9"], "no order"),
                                            ("Plant2/Lines/L2/Queue", ["SO-2001"], "can't make"),
                                            ("Plant2/KPI/OEE", 1.0, "read-only")])
def test_bad_writes_are_not_taken_and_are_not_interlock_events(plant, node, value, why):
    out = write(plant, node, value)
    assert why in out["error"] and "events" not in out and "refused" not in out["error"]


def test_a_save_restores_the_plan_and_serves_it_again(plant):
    write(plant, "Plant2/Lines/L1/Queue", ["SO-2001"])
    plant.save("i1", "s1", str(Path(os.environ["ARENA_SAVES"]) / "plant-opcua" / "s1"))
    plant.start("i2", config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert plant.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    assert plant.call_tool("i2", "opcua_read", {"nodes": ["Plant2/Lines/L1/Queue"]}, 0)["result"]["values"] == \
        {"Plant2/Lines/L1/Queue": ["SO-2001"]}


def test_describe_over_the_protocol_lists_the_bridge_tools():
    req = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "describe", "params": {}})
    out = subprocess.run(["python", "-m", "plant_opcua"], input=req + "\n", capture_output=True, text=True,
                         cwd=PACKAGE, timeout=60, check=True)
    assert [t["name"] for t in json.loads(out.stdout)["result"]["tools"]] == [
        "opcua_browse", "opcua_read", "opcua_write", "plant_scenario"]
