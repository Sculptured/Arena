"""The discrete-event model: deterministic per (seed, replication), common random numbers across
configs, and losses that move the way a plant's do."""
import json
import subprocess
import sys
from pathlib import Path

from conftest import DATA, config
from line_sim import des, plant

PACKAGE = Path(__file__).resolve().parents[1]
BASE = {k: v for k, v in config().items() if k != "study"}


def line2(changes: dict, rep: int) -> dict:
    p = plant.applied(BASE, plant.full(BASE, changes))
    return next(x for x in des.replicate(p, BASE["seed"], rep)["lines"] if x["id"] == "line2")


def test_a_replication_is_the_same_every_time_and_in_another_process():
    a = des.replicate(BASE, 2026, 3)
    assert a == des.replicate(BASE, 2026, 3)
    code = ("import json, sys; from line_sim import des; "
            "print(json.dumps(des.replicate(json.loads(open(sys.argv[1]).read()), 2026, 3)))")
    out = subprocess.run([sys.executable, "-c", code, str(DATA)], cwd=PACKAGE, capture_output=True,
                         text=True, check=True)
    assert json.loads(out.stdout) == json.loads(json.dumps(a))


def test_replications_differ_and_so_do_seeds():
    assert des.replicate(BASE, 2026, 0) != des.replicate(BASE, 2026, 1)
    assert des.replicate(BASE, 2026, 0) != des.replicate(BASE, 7, 0)


def test_common_random_numbers_make_a_real_improvement_win_on_every_replication():
    move = {"line1.crew": [1, 2, 1, 1], "line2.crew": [2, 2, 1, 1]}
    for rep in range(5):
        assert line2(move, rep)["good"] > line2({}, rep)["good"]


def test_a_change_to_one_line_leaves_the_other_lines_failure_draws_alone():
    """Each station draws from its own stream: Line 1's time to its first failure is the same."""
    p1 = plant.applied(BASE, plant.full(BASE, {"line2.batch_size": 300}))
    a = des.Sim(BASE, 2026, 0).lines[0].stations[0].ttf
    b = des.Sim(p1, 2026, 0).lines[0].stations[0].ttf
    assert a == b


def test_bigger_campaigns_cut_changeover_time_and_raise_output():
    small, big = line2({"line2.batch_size": 60}, 0), line2({"line2.batch_size": 240}, 0)
    changeover = [r["stations"][0]["share"]["changeover"] for r in (small, big)]
    assert changeover[1] < changeover[0] / 2 and big["good"] > small["good"]
    assert big["campaign_cycle_s"] > small["campaign_cycle_s"]


def test_machines_that_never_fail_are_never_down():
    p = plant.applied(BASE, plant.current(BASE))
    for line in p["lines"]:
        for st in line["stations"]:
            st.pop("mtbf_h", None)
    out = des.replicate(p, 2026, 0)
    downs = [s["share"]["down"] for x in out["lines"] for s in x["stations"]]
    assert downs == [0.0] * len(downs)
    assert out["lines"][1]["good"] > line2({}, 0)["good"]


def test_time_shares_add_up_and_pm_takes_the_hours_asked():
    r = line2({"line2.pm_hours": 8}, 0)
    for st in r["stations"]:
        assert abs(sum(st["share"].values()) - 1.0) < 1e-6
    assert abs(r["pm_s"] / 3600 - 8) < 0.5  # PM waits for a technician at times, never less than asked


def test_pm_factor_is_one_at_four_hours_and_grows_with_pm():
    assert abs(des.pm_factor(4) - 1.0) < 1e-12
    assert des.pm_factor(2) < 1.0 < des.pm_factor(8)
