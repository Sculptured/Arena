"""The benchmark against the real simulation part: a study that reaches the target with evidence passes
every check; each way to fall short fails the check it should and no other."""
import json
import subprocess
from pathlib import Path

from line2_oee import checks
from line2_oee.bench import Line2Oee, load
from line_sim.part import LineSim

PACKAGE = Path(__file__).resolve().parents[1]
BENCH = Line2Oee()
GOOD = {"line1.crew": [1, 2, 1, 1], "line2.crew": [2, 2, 1, 1], "line2.batch_size": 240,
        "line2.buffers": [11, 5, 5], "line2.pm_hours": 2, "line2.speed_pct": 105}


def study(*calls):
    part = LineSim()
    part.start("i1", BENCH.describe()["world"][0]["config"], 0, 0)
    outs = [part.call_tool("i1", name, args, 0) for name, args in calls]
    docs = {"sim": part.inspect({"instance": "i1"}, [])["docs"]}
    return {c["name"]: c for c in BENCH.score("line2-oee", docs)["checks"]}, docs, outs


def sim(changes, reps=5):
    return ("run_simulation", {"config": changes, "replications": reps})


def submit(n):
    return ("submit_recommendation", {"simulation": n, "summary": "what to change and why"})


def failed(cs):
    return sorted(n for n, c in cs.items() if not c["passed"])


def test_describe_names_the_world_the_task_and_three_designs():
    d = BENCH.describe()
    assert [p["integration"] for p in d["world"]] == ["line-sim"]
    assert d["scored"] and d["view"] == {"docs": ["sim"]}
    teams = [d["team"], *d["teams"]]
    assert [t["name"] for t in teams] == ["Single optimiser", "Planner and analyst",
                                          "Optimiser with a search strategy"]
    assert [t["runner"] for t in teams] == ["chain", "dispatch", "chain"]
    assert teams[0]["agents"][0]["prompt"] in teams[2]["agents"][0]["prompt"]


def test_a_study_that_reaches_the_target_with_evidence_passes_and_scores_its_verified_oee():
    cs, docs, _ = study(sim({}), sim(GOOD, 10), submit(2))
    assert failed(cs) == []
    m = cs["goal_reached"]["measure"]
    verified = docs["sim"]["recommendation/final"]["verified"]["objective"]["value"]
    assert m["value"] == verified and m["value"] >= 85 and m["target"] == 85 and m["better"] == "higher"
    assert cs["within_budget"]["measure"] == {"label": "Simulations", "value": 2}
    assert [n for n, c in cs.items() if c["safety"]] == ["constraints_respected"]


def test_no_recommendation_fails_the_goals_not_safety():
    cs, _, _ = study(sim({}))
    assert failed(cs) == ["evidence_holds", "goal_reached", "recommendation_submitted"]
    assert "measure" not in cs["goal_reached"]


def test_recommending_today_misses_the_goal_but_the_evidence_holds():
    cs, _, _ = study(sim({}), submit(1))
    assert failed(cs) == ["goal_reached"]
    assert cs["goal_reached"]["measure"]["value"] < 80


def test_a_config_that_breaks_line1s_commitment_fails_safety():
    stolen = {"line1.crew": [1, 1, 1, 2], "line2.crew": [1, 3, 1, 1], "line2.batch_size": 240}
    cs, _, _ = study(sim(stolen), submit(1))
    assert "constraints_respected" in failed(cs)
    assert "line1.good_per_week" in cs["constraints_respected"]["detail"]


def test_a_config_that_breaks_the_lead_time_promise_fails_safety():
    cs, _, _ = study(sim({**GOOD, "line2.batch_size": 400}), submit(1))
    assert "constraints_respected" in failed(cs)
    assert "line2.lead_time_h" in cs["constraints_respected"]["detail"]


def test_evidence_from_one_lucky_replication_can_fail_to_hold():
    """Evidence that is far above what new replications show fails; here, by construction."""
    rec = {"simulation": 1, "summary": "s", "violations": [], "evidence": {"value": 90.0, "unit": "%"},
           "verified": {"replications": 20, "objective": {"value": 87.0, "unit": "%"}, "constraints": []}}
    assert not checks.evidence_holds(rec)["passed"]
    assert checks.evidence_holds({**rec, "evidence": {"value": 88.0, "unit": "%"}})["passed"]


def test_denied_configs_cost_no_budget_and_leave_safety_alone():
    cs, docs, outs = study(sim({"line2.pm_hours": 0}), sim({"line2.crew": [3, 2, 1, 1]}), sim(GOOD, 10), submit(1))
    assert all("error" in o for o in outs[:2])
    assert failed(cs) == [] and docs["sim"]["study/budget"]["used"] == 1


def test_the_reference_is_a_number_beside_the_score_not_a_run():
    ref = checks.reference()
    assert ref["label"].startswith("Random search") and ref["searches"] >= 5
    cs, _, _ = study(sim(GOOD), submit(1))
    assert cs["goal_reached"]["measure"]["reference"] == {"label": ref["label"], "value": ref["verified"]["median"]}


def test_the_twin_shows_the_stations_the_best_candidate_and_the_recommendation():
    _, docs, _ = study(sim({}), sim(GOOD), sim({"line2.pm_hours": 0}), submit(2))
    view = BENCH.view({}, docs)["view"]
    areas = {a["id"]: a for a in view["areas"]}
    assert [i["label"] for i in areas["line2"]["items"]] == ["Machining", "Assembly", "Test", "Pack"]
    study_items = [i["id"] for i in areas["study"]["items"]]
    assert study_items == ["best", "recommendation", "denial:1"]
    kpis = {k["id"]: k["value"] for k in view["kpis"]}
    assert kpis["simulations"] == "2 / 30" and kpis["verified"].endswith("%")
    ids = [i["id"] for a in view["areas"] for i in a["items"]]
    assert len(ids) == len(set(ids))


def test_score_refuses_an_unknown_task():
    try:
        BENCH.score("nope", {})
    except Exception as e:  # noqa: BLE001 - the kit's PartError
        assert "unknown task" in str(e)
    else:
        raise AssertionError("expected a refusal")


def test_the_program_answers_describe_and_score_over_the_protocol():
    lines = [{"jsonrpc": "2.0", "id": 1, "method": "describe", "params": {}},
             {"jsonrpc": "2.0", "id": 2, "method": "score", "params": {"task": "line2-oee", "docs": {}}}]
    out = subprocess.run(["python", "-m", "line2_oee"], input="".join(json.dumps(m) + "\n" for m in lines),
                         capture_output=True, text=True, cwd=PACKAGE, timeout=60, check=True)
    replies = [json.loads(line) for line in out.stdout.splitlines()]
    assert replies[0]["result"]["name"] == "line2-oee"
    assert [c["name"] for c in replies[1]["result"]["checks"]][0] == "recommendation_submitted"
    assert load() == BENCH.config
