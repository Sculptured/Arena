"""The shift's scene pieces without Isaac Sim: the welding arm's cycle on the UR10e's own joints (as its NVIDIA asset
declares them, in tests/data), the AMRs' rounds, the boards, and the camera's cuts to the fronts."""
import json
from pathlib import Path

import numpy as np
import pytest

from isaac_part import showcase_camera as camera
from isaac_part import showcase_robot as robot
from isaac_part import showcase_shift_layout as plan
from isaac_part import showcase_shift_screens as screens

JOINTS = Path(__file__).parent / "data" / "ur10e_joints.json"


def ur10e() -> robot.Chain:
    data = json.loads(JOINTS.read_text())
    joints = [robot.Joint(j["parent"], j["child"], np.array(j["j0"]), np.array(j["j1_inv"]), np.array(j["axis"]),
                          j["lower"], j["upper"]) for j in (data[n] for n in plan.UR_JOINTS)]
    return robot.Chain(np.eye(4), joints, robot.pose((0.0, 0.0, plan.TORCH)))


def test_the_arm_reaches_every_seam_point_torch_down_and_parks():
    chain = ur10e()
    cycle = plan.weld_cycle(chain)
    for (phase, (dx, dy, dz), _), q in zip(plan.WELD_PATH, cycle.motion.angles):
        tip = chain.tool_pose(q)
        want = np.array([plan.TABLE[0] + dx, plan.TABLE[1] + dy, plan.TABLE[2] + dz]) - np.array(plan.ARM_BASE)
        assert np.allclose(tip[:3, 3], want, atol=2e-3), phase
        assert tip[2, 2] == pytest.approx(-1.0, abs=1e-2), "the torch points down"
    assert cycle.welding == ((0.12, 0.42), (0.64, 0.88))
    assert plan.burning(cycle, 0.3) and not plan.burning(cycle, 0.5)
    assert chain.tool_pose(cycle.park)[2, 3] == pytest.approx(0.85, abs=2e-3)


def test_the_amrs_keep_to_their_rounds():
    rounds = plan.amr_rounds()
    for r in rounds:
        a, b = r.pose(0.0), r.pose(r.total)
        assert np.allclose(a[:2], b[:2], atol=1e-6)
        xs = [r.pose(t)[0] for t in np.linspace(0, r.total, 200)]
        assert min(xs) > -17.0 and max(xs) < plan.CELL[0]
    assert set(plan.shots()) >= {"cell_r1", "cell_r1_close", "energy_board", "gateway", "dock_board", "amr"}


def test_the_boards_follow_the_signage():
    calm = screens.energy({"kw": 1525.0}, {})
    assert calm.bar[2] is None and calm.rows[1][1] == "--"
    over = screens.energy({"kw": 1525.0}, {"dr": {"value": "active", "number": 1200}})
    assert over.banner == "DEMAND RESPONSE ACTIVE" and over.color == screens.RED and over.rows[0][2] == screens.RED
    held = screens.energy({"kw": 1163.0}, {"dr": {"value": "active", "number": 1200}})
    assert held.color == screens.GREEN
    assert screens.gateway({"ot": {"value": "intrusion"}}).color == screens.RED
    assert screens.dock({"truck": {"value": "INB-0310-1", "number": 6}}).rows[2][1] == "+6 h"
    assert screens.cell({"status": "safe-stopped"}, {}).banner == "R1 SAFE-STOPPED"
    assert "simulated" in screens.energy({"kw": 1.0}, {}).rows[2][1]


def test_the_boards_paint(tmp_path):
    pytest.importorskip("PIL")
    b = screens.Boards(tmp_path)
    one = b.file_for(screens.dock({}))
    assert Path(one).exists() and b.file_for(screens.dock({})) == one


def test_fronts_cut_to_what_shows_them_each_in_turn():
    shots = {**{name: None for name, _ in camera.SHIFT_ESTABLISHING}, **plan.shots(), "wide": None}
    d = camera.Director(shots, "auto", camera.SHIFT_ESTABLISHING)
    d.update([], 0.0)
    cues = [{"kind": "shift", "data": {"cue": c}} for c in ("intrusion", "truck")]
    d.update(cues, 1.0)
    assert d.current == "gateway"
    names = [d.update([], t)[0] for t in np.arange(1.0, 30.0, 0.5)]
    order = [n for i, n in enumerate(names) if i == 0 or names[i - 1] is not n]
    assert order[:3] == [shots["gateway"], shots["cell_r1"], shots["dock_board"]]
