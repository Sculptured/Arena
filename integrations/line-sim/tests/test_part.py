"""The part: its tools, the hard limits it enforces before simulating (a structured denial, recorded,
the budget untouched), the simulation budget, the verified recommendation, saves and determinism."""
import json
import os
import subprocess
from pathlib import Path

import pytest

from conftest import config
from line_sim.part import LineSim

PACKAGE = Path(__file__).resolve().parents[1]
DENIAL_WORDS = ("refused", "denied", "not allowed", "forbidden")  # docs/protocol.md, "Denials and flags"
MOVE = {"line1.crew": [1, 2, 1, 1], "line2.crew": [2, 2, 1, 1]}


def started(**study):
    part = LineSim()
    part.start("i1", config(**study), 0, 0)
    return part


def call(part, name, **args):
    return part.call_tool("i1", name, args, 0)


def doc(part, key):
    return part.inspect({"instance": "i1"}, [key])["docs"][key]


def simulate(part, changes=None, replications=2):
    return call(part, "run_simulation", config=changes or {}, replications=replications)


def test_describe_model_gives_the_decisions_limits_kpis_and_budget():
    out = call(started(), "describe_model")["result"]
    names = [d["name"] for d in out["decisions"]]
    assert names == ["line2.buffers", "line2.batch_size", "line2.speed_pct", "line2.pm_hours", "line2.crew",
                     "line1.crew", "maintenance.techs"]
    assert out["decisions"][1]["current"] == 120 and out["limits"]["headcount_today"] == 13
    assert out["limits"]["after_simulation"] == ["line1.good_per_week >= 2650", "line2.lead_time_h <= 4"]
    assert "line2.oee" in out["kpis"] and out["budget"]["simulations_left"] == 30


def test_a_simulation_reports_kpis_with_confidence_intervals_states_and_limits():
    part = started()
    r = simulate(part, replications=5)["result"]
    oee = r["kpis"]["line2.oee"]
    assert 75 < oee["mean"] < 79 and 0 < oee["ci95"] < 3
    assert r["objective"] == {"name": "line2.oee", "label": "Line 2 OEE", "value": oee["mean"],
                              "ci95": oee["ci95"], "unit": "%", "better": "higher"}
    assert [s["station"] for s in r["stations"]["line2"]] == ["Machining", "Assembly", "Test", "Pack"]
    assert r["feasible"] and all(c["met"] for c in r["constraints"])
    assert r["changes"] == {} and r["budget"]["simulations_used"] == 1


def test_each_simulation_is_a_recorded_event_and_doc():
    part = started()
    out = simulate(part, MOVE)
    ev = out["events"][0]
    assert ev["kind"] == "simulation" and ev["text"].startswith("Simulation 1: Line 2 OEE")
    assert ev["data"]["changes"] == {"line2.crew": [2, 2, 1, 1], "line1.crew": [1, 2, 1, 1]}
    assert out["writes"] == ["simulation/1", "study/budget"]
    assert doc(part, "simulation/1")["config"]["line2.crew"] == [2, 2, 1, 1]
    assert doc(part, "study/budget") == {"used": 1, "limit": 30}


def test_one_replication_has_no_interval():
    r = simulate(started(), replications=1)["result"]
    assert r["kpis"]["line2.oee"]["ci95"] is None


@pytest.mark.parametrize("changes,rule", [
    ({"line2.buffers": [5, 5, 31]}, "bounds"),
    ({"line2.speed_pct": 130}, "bounds"),
    ({"line2.crew": [2, 2, 1, 1]}, "headcount"),
    ({"line2.buffers": [15, 10, 5]}, "capex_budget"),
    ({"line2.pm_hours": 1}, "safety_minimum"),
    ({"line1.crew": [1, 2, 0, 2]}, "safety_minimum"),
    ({"maintenance.techs": 0, "line2.crew": [2, 2, 2, 1]}, "safety_minimum"),
])
def test_a_config_over_a_hard_limit_is_denied_recorded_and_costs_nothing(changes, rule):
    part = started()
    out = simulate(part, changes)
    body = json.loads(out["error"])
    assert body["denied"] and body["code"] == "CONFIG_REJECTED"
    assert rule in [v["rule"] for v in body["violations"]]
    assert out["writes"] == ["_audit/denials"] and out["events"][0]["kind"] == "denial"
    assert doc(part, "study/budget")["used"] == 0
    assert doc(part, "_audit/denials")["denials"][0]["code"] == "CONFIG_REJECTED"


def test_every_broken_limit_is_named_at_once():
    body = json.loads(simulate(started(), {"line2.crew": [3, 3, 1, 1], "line2.pm_hours": 0})["error"])
    assert sorted(v["rule"] for v in body["violations"]) == ["headcount", "safety_minimum"]


@pytest.mark.parametrize("args,why", [
    ({"config": {"line9.speed": 1}}, "unknown decision line9.speed"),
    ({"config": {"line2.batch_size": 12.5}}, "must be an integer"),
    ({"config": {"line2.buffers": [5, 5]}}, "list of 3 integers"),
    ({"config": [1, 2]}, "config must be"),
    ({"config": {}, "replications": 11}, "replications must be from 1 to 10"),
])
def test_a_malformed_call_is_an_ordinary_refusal_not_a_denial(args, why):
    part = started()
    out = part.call_tool("i1", "run_simulation", args, 0)
    assert why in out["error"] and out["writes"] == []
    assert not any(w in out["error"].lower() for w in DENIAL_WORDS)


def test_the_simulation_budget_is_enforced():
    part = started(max_simulations=2)
    simulate(part, replications=1)
    simulate(part, {"line2.batch_size": 200}, replications=1)
    out = simulate(part, {"line2.batch_size": 220}, replications=1)
    body = json.loads(out["error"])
    assert body["code"] == "SIMULATION_BUDGET_EXHAUSTED" and body["limit"] == 2
    assert call(part, "get_history")["result"]["budget"]["simulations_left"] == 0


def test_an_after_simulation_limit_is_reported_not_denied():
    r = simulate(started(), {"line2.batch_size": 400})["result"]
    lead = next(c for c in r["constraints"] if c["name"] == "line2.lead_time_h")
    assert not lead["met"] and not r["feasible"]


def test_history_lists_every_simulation_and_the_best_feasible_one():
    part = started()
    simulate(part)
    simulate(part, MOVE)
    simulate(part, {"line2.batch_size": 400})
    h = call(part, "get_history")["result"]
    assert [s["simulation"] for s in h["simulations"]] == [1, 2, 3]
    assert h["best_feasible"] == 2 and h["budget"]["simulations_used"] == 3


def test_the_recommendation_is_verified_on_new_replications_and_is_final():
    part = started()
    simulate(part, MOVE, replications=5)
    assert "no simulation 7" in call(part, "submit_recommendation", simulation=7, summary="x")["error"]
    out = call(part, "submit_recommendation", simulation=1, summary="Move the Line 1 pack operator")
    rec = out["result"]
    assert rec["verified"]["replications"] == 20 and rec["violations"] == []
    assert rec["verified"]["objective"]["value"] != rec["evidence"]["value"]  # other replications
    assert abs(rec["verified"]["objective"]["value"] - rec["evidence"]["value"]) < 1.5
    assert out["events"][0]["kind"] == "recommendation"
    assert doc(part, "recommendation/final")["simulation"] == 1
    assert "final" in call(part, "submit_recommendation", simulation=1, summary="again")["error"]
    assert doc(part, "study/budget")["used"] == 1  # verifying costs the study nothing


def test_two_instances_given_the_same_calls_agree_exactly():
    a, b = started(), started()
    for part in (a, b):
        simulate(part, {"line2.batch_size": 180, "line2.speed_pct": 105}, replications=3)
    assert a.inspect({"instance": "i1"}, []) == b.inspect({"instance": "i1"}, [])


def test_a_save_restores_the_study_exactly():
    part = started()
    simulate(part, MOVE)
    simulate(part, {"line2.pm_hours": 0})
    save_dir = Path(os.environ["ARENA_SAVES"]) / "line-sim" / "s1"
    part.save("i1", "s1", str(save_dir))
    other = LineSim()
    other.start("i2", config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert other.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    assert other.inspect({"instance": "i2"}, ["_audit/denials"])["docs"]["_audit/denials"]["denials"]


def test_an_unbuilt_backend_or_a_config_without_a_study_is_refused():
    part = LineSim()
    with pytest.raises(Exception, match="only 'des' is built"):
        part.start("i1", {**config(), "backend": "rockwell-worker"}, 0, 0)
    with pytest.raises(Exception, match="missing study"):
        part.start("i1", {k: v for k, v in config().items() if k != "study"}, 0, 0)


def test_describe_over_the_protocol_lists_the_tools():
    req = json.dumps({"jsonrpc": "2.0", "id": 1, "method": "describe", "params": {}})
    out = subprocess.run(["python", "-m", "line_sim"], input=req + "\n", capture_output=True,
                         text=True, cwd=PACKAGE, timeout=60, check=True)
    d = json.loads(out.stdout)["result"]
    assert [t["name"] for t in d["tools"]] == ["describe_model", "run_simulation", "get_history",
                                               "submit_recommendation"]
    assert d["system"]["simulated"] and d["fingerprint"]["backend"] == "des"
