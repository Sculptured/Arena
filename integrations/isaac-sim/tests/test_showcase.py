"""The showcase scene's plain parts, no GPU: geometry, arm kinematics, the flow of parts, the camera plan. Nothing here
may feed back into the plant, so this file also runs the stand-in plant with the scene's config set and compares."""
import math

import numpy as np
import pytest

from isaac_part import config, showcase_camera as camera, showcase_mesh as mesh, showcase_motion as motion
from isaac_part import showcase_layout as layout, showcase_press, showcase_robot as robot
from isaac_part.backends import BACKENDS
from isaac_part.part import IsaacSim


def test_box_faces_point_outward_and_chamfers_stay_inside():
    pieces = mesh.box_pieces((2.0, 3.0, 4.0), 0.1)
    assert len(pieces) == 6 + 12 + 8
    for points, normals in pieces:
        assert np.all(np.abs(points) <= np.array([1.0, 1.5, 2.0]) + 1e-9)
        assert np.linalg.norm(normals, axis=1) == pytest.approx(np.ones(len(normals)))
        facing = np.cross(points[1] - points[0], points[2] - points[0])
        assert np.dot(facing, points.mean(axis=0)) > 0


@pytest.mark.parametrize("axis, along", [("X", 0), ("Y", 1), ("Z", 2)])
def test_cylinder_lies_along_its_axis(axis, along):
    m = mesh.Merged()
    m.cylinder((0, 0, 0), 0.5, 3.0, axis, 12, 0.02)
    low, high = m.bounds()
    assert high[along] == pytest.approx(1.5)
    assert low[along] == pytest.approx(-1.5)
    assert max(abs(c) for i, c in enumerate(high) if i != along) <= 0.5 + 1e-9


def test_merged_indices_cover_every_point():
    m = mesh.Merged()
    m.box((1, 2, 3), (1, 1, 1), 0.1, yaw_deg=30)
    m.cylinder((0, 0, 0), 1, 1)
    a = m.arrays()
    assert a["counts"].sum() == len(a["indices"]) == len(a["points"]) == len(a["normals"]) == len(a["uvs"])
    assert a["indices"].max() == len(a["points"]) - 1


def test_stamped_panel_has_a_flat_flange_and_a_drawn_pocket():
    points, normals, faces = mesh.stamped_panel(1.0, 0.8, 0.1, n=20)
    assert points[:, 2].max() <= 1e-9 + 0.05
    assert points[:, 2].min() < -0.05
    assert abs(points[0, 2]) < 1e-6  # the corner is the flange
    assert np.all(normals[:, 2] > 0)
    assert faces.max() < len(points)


@pytest.mark.parametrize("scale", [1.0, 0.9, 0.8])
def test_press_slide_clears_the_die_and_the_crown(scale):
    p = showcase_press.build(scale)
    low, high = p.groups["slide"]["die"].bounds()
    assert low[2] == pytest.approx(p.slide_bdc_z, abs=1e-6)  # the upper die shuts on the lower one
    top_of_slide = p.groups["slide"]["body"].bounds()[1][2] + p.stroke
    assert top_of_slide < showcase_press.COLUMN_TOP * scale
    pitman_top = p.groups["slide"]["steel"].bounds()[1][2] + p.stroke
    assert pitman_top < showcase_press.CROWN_TOP * scale  # stays hidden in the crown at the top of the stroke
    assert p.groups["frame"]["body"].bounds()[1][1] < 2.7 * scale


def _arm():
    """A 6-axis arm the size of the real one: shoulder, elbow, spherical wrist."""
    quat = (1, 0, 0, 0)
    parts = [("base", "l1", (0, 0, 0.67), "Z"), ("l1", "l2", (0.31, 0, 0), "Y"), ("l2", "l3", (0, 0, 1.28), "Y"),
             ("l3", "l4", (0.5, 0, 0.23), "X"), ("l4", "l5", (0.8, 0, 0), "Y"), ("l5", "l6", (0.2, 0, 0), "X")]
    return robot.Chain(np.eye(4), [robot.joint_from_usd(a, b, p, quat, (0, 0, 0), quat, axis, -180, 180)
                                   for a, b, p, axis in parts])


def _fanuc():
    """The FANUC R-2000iC/165F as its 6.2 asset declares it (read 2026-11-05)."""
    flat, flip, turn = (1, 0, 0, 0), (0, 1, 0, 0), (0, 0, 0, 1)
    rows = [("robot_base", "J1_link", (0, 0, 0.67), flat, "Z", -185, 185),
            ("J1_link", "J2_link", (0.312, 0, 0), flat, "Y", -60, 76),
            ("J2_link", "J3_link", (0, 0, 1.075), flip, "Y", -79, 180),
            ("J3_link", "J4_link", (0, 0, 0.225), turn, "X", -360, 360),
            ("J4_link", "J5_link", (1.28, 0, 0), flip, "Y", -125, 125),
            ("J5_link", "J6_link", (0, 0, 0), turn, "X", -360, 360)]
    return robot.Chain(np.eye(4), [robot.joint_from_usd(a, b, p, q, (0, 0, 0), q, axis, lo, hi)
                                   for a, b, p, q, axis, lo, hi in rows])


class _Layout:
    """The factory layout's three press lines, as `line_layouts` reads them."""
    LINES = {"LINE-A": {"machine": "PR-A1"}, "LINE-B": {"machine": "PR-B1"}, "LINE-C": {"machine": "PR-C1"}}

    @staticmethod
    def line_geometry(lid):
        y, s = {"LINE-A": (13.0, 1.0), "LINE-B": (6.5, 0.9), "LINE-C": (0.0, 0.8)}[lid]
        return {"press": (0.0, y), "dims": {"scale": s}}


@pytest.mark.parametrize("line", layout.line_layouts(_Layout), ids=lambda l: l.line)
def test_the_arm_can_serve_every_line(line):
    arm = _fanuc().with_world_aligned_tool((layout.TOOL_BOOM, 0, -layout.TOOL_DROP))
    for make, base in ((layout.loader_motion, line.loader_base()), (layout.unloader_motion, line.unloader_base())):
        cycle = make(arm, line)
        steps = np.array([cycle.at(p) for p in np.linspace(0, 1, 201)])
        assert np.all(np.abs(np.diff(steps, axis=0)) < 0.6)  # no joint jumps between neighbouring phases
        assert np.all(steps >= arm.lower - 1e-6) and np.all(steps <= arm.upper + 1e-6)
        tips = np.array([arm.tool_pose(q)[:3, 3] + base for q in steps])
        assert tips[:, 2].min() > layout.BELT_TOP  # never through a belt


def test_the_booms_clear_the_slide_as_it_moves():
    """Whenever a boom's tip is under the slide, it is below the upper die at that point of the stroke."""
    arm = _fanuc().with_world_aligned_tool((layout.TOOL_BOOM, 0, -layout.TOOL_DROP))
    for line in layout.line_layouts(_Layout):
        half_depth, half_width = 1.1 * line.scale, 1.4 * line.scale
        p = showcase_press.build(line.scale)
        for make, base in ((layout.loader_motion, line.loader_base()), (layout.unloader_motion, line.unloader_base())):
            cycle = make(arm, line)
            for phase in np.linspace(0, 1, 200, endpoint=False):
                tip = arm.tool_pose(cycle.at(phase))[:3, 3] + base
                if abs(tip[0] - line.cx) < half_depth and abs(tip[1] - line.cy) < half_width:
                    bottom = p.slide_bdc_z + p.stroke * (1 + math.cos(2 * math.pi * phase)) / 2
                    assert tip[2] + 0.03 < bottom, f"{line.line} phase {phase:.3f}: tip {tip}, die at {bottom:.2f}"


def test_the_shots_stand_on_the_camera_plan():
    all_shots = layout.shots(_Layout)
    plan = (camera.ESTABLISHING, camera.FIRST_WARNING, camera.SECOND_WARNING, camera.TRIP, camera.AFTERMATH,
            *camera.CUTS.values())
    assert {n for steps in plan for n, _ in steps} <= set(all_shots)


def test_inverse_kinematics_reaches_a_tool_pose():
    arm = _arm().with_world_aligned_tool((1.2, 0, -0.2))
    target = robot.pose((2.6, 0.5, 1.4), robot.yaw_rotation(0.2))
    q = robot.solve(arm, target, np.array([0.2, 0.3, -0.3, 0, 0.1, 0]))
    assert arm.tool_pose(q)[:3, 3] == pytest.approx(target[:3, 3], abs=1e-3)
    assert arm.tool_pose(q)[:3, :3] == pytest.approx(target[:3, :3], abs=1e-2)


def test_inverse_kinematics_refuses_what_it_cannot_reach():
    arm = _arm().with_world_aligned_tool((0.2, 0, 0))
    with pytest.raises(ValueError, match="no joint angles"):
        robot.solve(arm, robot.pose((9.0, 0, 1.0)), np.zeros(6), iterations=30)


def test_cycle_motion_passes_its_keys_and_wraps():
    keys = [(0.0, np.zeros(2)), (0.5, np.ones(2)), (0.9, np.array([2.0, 0.0]))]
    m = robot.CycleMotion(keys)
    assert m.at(0.5) == pytest.approx([1, 1])
    assert m.at(1.5) == pytest.approx([1, 1])
    assert m.at(0.25) == pytest.approx([0.5, 0.5])
    assert m.at(0.95) == pytest.approx([1.0, 0.0])


def _flow():
    return motion.LineFlow(60 / 7, motion.Belt(0.0, 5.0, motion.BLANK_GAP), motion.Belt(0.0, 2.5, motion.PART_GAP))


def _run(flow, seconds, cycle, spm=7.0):
    dt = 1 / 30
    for _ in range(int(seconds / dt)):
        cycle += spm / 60 * dt
        flow.update(cycle, dt)
    return cycle


def test_parts_flow_into_the_stillage_while_the_press_runs():
    flow = _flow()
    _run(flow, 120, 0.2)
    assert flow.strokes == pytest.approx(14, abs=2)
    assert flow.stillage >= flow.strokes - 2  # what was stamped went on to the stillage
    assert 1 <= len(flow.infeed.parts) <= 3


def test_a_stopped_press_backs_up_the_infeed_and_stops_output():
    flow = _flow()
    cycle = _run(flow, 60, 0.2)
    running = len(flow.infeed.parts)
    strokes = flow.strokes
    dt = 1 / 30
    for _ in range(int(60 / dt)):
        flow.update(cycle, dt)  # the crank stands still
    assert len(flow.infeed.parts) > running  # queued up to the gate
    assert flow.infeed.parts[0] == pytest.approx(flow.infeed.stop)
    gaps = -np.diff(flow.infeed.parts)
    assert gaps == pytest.approx(np.full(len(gaps), motion.BLANK_GAP))
    assert not flow.outfeed.parts  # the outfeed ran empty
    final = flow.stillage
    for _ in range(int(30 / dt)):
        flow.update(cycle, dt)
    assert flow.stillage == final
    assert flow.strokes == strokes


def test_a_warm_start_has_parts_everywhere_without_counting_real_time():
    flow = _flow()
    flow.warm(6)
    assert flow.infeed.parts and flow.stillage > 0
    assert flow.cycle is None
    before = flow.stillage
    flow.update(123.25, 1 / 30)  # the first real reading is not a jump of 100 cycles
    assert flow.stillage <= before + 1


def test_shake_follows_the_vibration_and_ignores_the_spin_up():
    assert motion.shake_amplitude(900.0, 1.0) == 0.0
    assert motion.shake_amplitude(1.0, 30.0) == 0.0
    mid, full = motion.shake_amplitude(6.0, 30.0), motion.shake_amplitude(11.2, 30.0)
    assert 0 < mid < full == pytest.approx(motion.SHAKE_MAX_M)
    assert motion.shake_amplitude(50.0, 30.0) == pytest.approx(full)
    assert all(abs(c) <= full for t in np.linspace(0, 5, 50) for c in motion.shake_offset(full, t))


def test_crank_cycle_is_the_factorys_crank():
    assert motion.crank_cycle(0.0, 42.0) == pytest.approx(0.5)  # shaft angle 0 is the ram at bottom dead centre
    assert motion.crank_cycle(2 * math.pi * 21, 42.0) == pytest.approx(1.0)


def test_round_loops_and_faces_where_it_goes():
    r = motion.Round([motion.Leg((0, 0), (10, 0), 5.0), motion.Leg((10, 0), (10, 10), 5.0, carrying=True),
                      motion.Leg((10, 10), (0, 0), 5.0, reverse=True)])
    assert r.pose(0.0)[:2] == pytest.approx((0, 0))
    assert r.pose(2.5)[2] == pytest.approx(0.0)
    assert r.pose(7.5)[:3] == pytest.approx((10, 5, 90))
    assert r.pose(7.5)[4] is True
    assert r.pose(15.0)[:2] == pytest.approx((0, 0))


def test_a_turn_on_the_spot_goes_the_short_way():
    r = motion.Round([motion.Leg((0, 0, 170.0), (0, 0, -170.0), 2.0), motion.Leg((0, 0), (5, 0), 2.0)])
    assert r.pose(1.0)[2] % 360 == pytest.approx(180.0)


def test_the_forklift_never_snaps_round():
    lines = layout.line_layouts(_Layout)
    r = layout.forklift_round(lines)
    yaws = [r.pose(t)[2] for t in np.arange(0, r.total, 0.05)]
    steps = [abs((b - a + 180) % 360 - 180) for a, b in zip(yaws, yaws[1:] + yaws[:1])]
    assert max(steps) < 5.0


def _shots():
    names = {n for plan in (camera.ESTABLISHING, camera.FIRST_WARNING, camera.SECOND_WARNING, camera.TRIP,
                            camera.AFTERMATH, *camera.CUTS.values()) for n, _ in plan}
    return {n: camera.Shot((0, 0, 5), (1, 1, 1), 20.0) for n in names}


def test_camera_establishes_then_cuts_to_the_failing_press_on_events():
    d = camera.Director(_shots())
    d.update([], 0.0)
    assert d.current == "wide"
    d.update([{"kind": "warning", "text": "PR-A1 main drive vibration 4.5 mm/s, above 4.5 (zone C)"}], 34.0)
    assert d.current == "press_a"
    d.update([{"kind": "alarm", "text": "ALARM PR-A1: main drive protective trip"}], 62.0)
    assert d.current == "press_a" and d.started == 62.0
    seen = []
    for t in np.arange(62.0, 95.0, 0.5):
        d.update([], float(t))
        seen.append(d.current)
    assert {"beacon_a", "infeed_a", "outfeed_a"} <= set(seen)  # the alarm, the backup and the empty outfeed
    assert "forklift" not in seen  # no idle cut-aways during the failure


def test_camera_leaves_other_news_alone_and_a_named_shot_stays():
    d = camera.Director(_shots())
    d.update([], 0.0)
    d.update([{"kind": "warning", "text": "PR-B1 something"}], 3.0)
    assert d.current == "wide"
    fixed = camera.Director(_shots(), "belt_c")
    assert fixed.update([{"kind": "alarm", "text": "ALARM PR-A1"}], 1.0)[0] is fixed.shots["belt_c"]


def test_scene_is_validated_and_defaults_to_the_factorys_own():
    assert config.parse_config({}).scene == "ridgeline"
    cfg = config.parse_config({"scene": "showcase", "record_height": 1080})
    assert (cfg.scene, cfg.record_height) == ("showcase", 1080)
    with pytest.raises(Exception, match="scene must be"):
        config.parse_config({"scene": "nope"})


def test_the_scene_config_changes_nothing_in_the_plant():
    """`scene` is visuals only: the stand-in plant's readings and events are the same with it."""
    def run(extra):
        part = IsaacSim(BACKENDS["standin"])
        part.start("p", {"scenario": True, **extra}, 7, 0)
        out = [part.advance("p", ms)["events"] for ms in range(5000, 70000, 5000)]
        state = part.inspect({"instance": "p"}, ["machine/PR-A1", "line/LINE-A"])
        part.stop("p")
        return out, state
    assert run({}) == run({"scene": "showcase"})
