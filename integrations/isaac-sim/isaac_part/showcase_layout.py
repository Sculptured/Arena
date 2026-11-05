"""Where the showcase's visuals stand and how its arms move, plain arithmetic. The presses stay where the plant's
layout has them (and the aisles its people and AMR walk stay clear); everything else in this file is only seen."""
from dataclasses import dataclass

import numpy as np

from . import showcase_motion as motion
from . import showcase_press as press
from . import showcase_robot as robot
from .showcase_camera import Shot

BELT_TOP = 0.78  # the conveyor asset's rollers
BELT_SEGMENT = 2.719  # its length; its origin is at its downstream end
BLANK_SIZE = (0.95, 0.75)
PEDESTAL_Z = 0.5
TOOL_BOOM = 1.45  # from the arm's last joint to the suction cups, so that the wrist stays out of the die
TOOL_DROP = 0.24  # the cups' height below that joint's axis
ARM_LINKS = ("J1_link", "J2_link", "J3_link", "J4_link", "J5_link", "J6_link")
SHOPFLOOR_LINES = ("LINE-A", "LINE-B", "LINE-C")


@dataclass(frozen=True)
class LineLayout:
    line: str
    machine: str
    cx: float
    cy: float
    scale: float

    def die(self) -> tuple:
        return (self.cx, self.cy, press.DIE_TOP * self.scale + 0.02)

    def loader_base(self) -> tuple:
        return (self.cx - 3.7, self.cy - 0.8, PEDESTAL_Z)

    def unloader_base(self) -> tuple:
        return (self.cx + 3.6, self.cy + 1.0, PEDESTAL_Z)

    def infeed_y(self) -> float:
        return self.cy

    def infeed_x(self) -> tuple:
        """From the washer's exit to where the first blank stops under the loader."""
        return (self.cx - 11.8, self.cx - 6.2)

    def infeed_origins(self) -> list:
        """The conveyor segments' origins, each at its downstream end."""
        end = self.cx - 5.95
        return [end, end - BELT_SEGMENT]

    def outfeed_y(self) -> float:
        return self.cy - 1.0

    def outfeed_x(self) -> tuple:
        """From where the unloader sets a part down to the end of the belt."""
        return (self.cx + 4.5, self.cx + 6.25)

    def outfeed_origin(self) -> float:
        return self.cx + 6.42

    def stillage(self) -> tuple:
        return (self.cx + 6.82, self.cy - 1.0)

    def washer(self) -> tuple:
        return (self.cx - 12.1, self.cy)

    def blank_stack(self) -> tuple:
        return (self.cx - 13.4, self.cy)


def line_layouts(layout) -> list:
    """Press lines A, B and C in the factory's own layout (`layout`, its module)."""
    out = []
    for lid in SHOPFLOOR_LINES:
        geo = layout.line_geometry(lid)
        out.append(LineLayout(lid, layout.LINES[lid]["machine"], geo["press"][0], geo["press"][1], geo["dims"]["scale"]))
    return out


def _tool(base, at, yaw_deg) -> np.ndarray:
    """The tool pose, in the arm's root frame, for cups at the world point `at`, with the boom along `yaw_deg`."""
    return robot.pose((at[0] - base[0], at[1] - base[1], at[2] - base[2]), robot.yaw_rotation(np.radians(yaw_deg)))


def _keys(arm, base, seed, waypoints) -> list:
    """Cycle keys from waypoints (phase, world point, boom yaw): each solved from the last, so the arm keeps one
    branch and its joints move the short way."""
    keys, q = [], np.asarray(seed, dtype=float)
    for phase, at, yaw in waypoints:
        q = robot.solve(arm, _tool(base, at, yaw), q)
        keys.append((phase, q.copy()))
    return sorted(keys, key=lambda k: k[0])


def loader_motion(arm, line: LineLayout) -> robot.CycleMotion:
    """Takes the blank at the head of the infeed, waits outside the window, sets it in the die as the slide tops out.
    The boom swings round over the side away from the operator."""
    base, die = line.loader_base(), line.die()
    x, y, z = die
    pick = (line.infeed_x()[1], line.infeed_y(), BELT_TOP + 0.03)
    above_die, front = (x, y, z + 0.2), (x - 1.65, y, z + 0.3)
    over_belt, swing = (pick[0], pick[1], pick[2] + 0.45), (x - 3.3, y + 2.2, 2.3)
    return robot.CycleMotion(_keys(arm, base, [0.1, 0.2, -0.2, 0.0, -0.1, 0.0], [
        (0.02, die, 0.0), (0.06, above_die, 0.0), (0.14, front, 0.0), (0.24, swing, 90.0),
        (0.34, over_belt, 180.0), (motion.PHASE_LOADER_GRIP - 0.01, pick, 180.0), (0.44, over_belt, 180.0),
        (0.56, swing, 90.0), (0.70, front, 0.0), (0.89, front, 0.0), (0.95, above_die, 0.0), (0.985, die, 0.0)]))


def unloader_motion(arm, line: LineLayout) -> robot.CycleMotion:
    """Reaches in as the slide rises, lifts the formed part out, sets it on the outfeed, waits."""
    base, die = line.unloader_base(), line.die()
    x, y, z = die
    drop = (line.outfeed_x()[0], line.outfeed_y(), BELT_TOP + 0.09)
    above_die, front = (x, y, z + 0.2), (x + 1.65, y, z + 0.3)
    over_belt, rest = (drop[0], drop[1], drop[2] + 0.5), (x + 1.75, y, z + 0.75)
    turn = (x + 3.2, y - 1.4, 2.3)
    return robot.CycleMotion(_keys(arm, base, [-2.67, 0.2, -0.2, 0.0, -0.1, 0.0], [
        (0.76, front, 180.0), (0.83, above_die, 180.0), (motion.PHASE_UNLOADER_GRIP - 0.01, die, 180.0),
        (0.88, above_die, 180.0), (0.94, front, 180.0), (0.985, turn, 225.0), (0.04, over_belt, 270.0),
        (motion.PHASE_UNLOADER_RELEASE - 0.01, drop, 270.0), (0.17, over_belt, 270.0), (0.32, rest, 180.0),
        (0.62, rest, 180.0)]))


def belt_flow(line: LineLayout, spm: float) -> motion.LineFlow:
    x0, x1 = line.infeed_x()
    ox0, ox1 = line.outfeed_x()
    return motion.LineFlow(60.0 / spm, motion.Belt(0.0, x1 - x0, motion.BLANK_GAP),
                           motion.Belt(0.0, ox1 - ox0, motion.PART_GAP))


def forklift_round(lines) -> motion.Round:
    """Blank pallets in from the south aisle along the line heads, and back empty."""
    west = min(l.cx for l in lines) - 15.0
    south, north = min(l.cy for l in lines) - 6.0, max(l.cy for l in lines)
    east = max(l.cx for l in lines) + 5.5
    leg = motion.Leg
    return motion.Round([
        leg((east, south), (west, south), 16.0, carrying=True),
        leg((west, south, 180.0), (west, south, 90.0), 2.5, carrying=True),
        leg((west, south), (west, north), 12.0, carrying=True),
        leg((west, north, 90.0), (west, north, 0.0), 2.5, carrying=True),
        leg((west, north, 0.0), (west, north, 0.0), 3.0, 0.1, 1.0, carrying=True),
        leg((west, north, 0.0), (west, north, 0.0), 2.0, 1.0, 0.1),
        leg((west, north, 0.0), (west, north, 90.0), 2.5),
        leg((west, north), (west, south), 12.0, reverse=True),
        leg((west, south, 90.0), (west, south, 180.0), 2.5),
        leg((west, south), (east, south), 16.0, reverse=True),
        leg((east, south, 180.0), (east, south, 180.0), 4.0),
    ])


def shots(layout) -> dict:
    """The camera plan's shots. Eyes stand inside the hall, at heights a person or a mast could really film from."""
    a, b, c = line_layouts(layout)
    ax, ay = a.cx, a.cy
    bx, by = b.cx, b.cy
    cx, cy = c.cx, c.cy
    return {
        "wide": Shot((-15.0, -9.5, 6.5), (0.5, 6.5, 1.8), 17.0, eye_end=(-13.0, -10.5, 6.0)),
        "wide_a": Shot((-14.5, ay - 9.5, 4.2), (ax, ay, 2.4), 18.0, eye_end=(-13.0, ay - 10.0, 4.0)),
        "robot_b": Shot((bx - 7.2, by - 4.2, 2.6), (bx - 2.6, by, 1.5), 22.0, eye_end=(bx - 6.2, by - 4.6, 2.5)),
        "belt_c": Shot((cx - 5.2, cy - 2.4, 1.7), (cx - 9.5, cy, 0.8), 22.0, eye_end=(cx - 5.8, cy - 2.6, 1.6)),
        "forklift": Shot((-8.0, min(l.cy for l in (a, b, c)) - 9.5, 2.0), (0, 0, 1.0), 20.0, follow="forklift"),
        "press_a_far": Shot((ax + 8.0, ay - 7.5, 3.2), (ax, ay, 3.2), 18.0, eye_end=(ax + 6.8, ay - 7.8, 3.0)),
        "press_a": Shot((ax - 6.5, ay - 9.5, 3.0), (ax - 0.5, ay, 3.0), 20.0, eye_end=(ax - 5.5, ay - 9.0, 2.8), shake=0.25),
        "press_a_low": Shot((ax - 4.4, ay - 6.8, 1.0), (ax, ay, 3.0), 15.0, eye_end=(ax - 3.8, ay - 6.6, 1.0), shake=0.4),
        "hmi_a": Shot((ax - 3.3, ay - 2.9, 1.65), (ax - 2.35, ay - 1.6, 1.42), 30.0, shake=0.1),
        "beacon_a": Shot((ax - 5.5, ay - 5.0, 3.2), (ax - 1.6, ay - 1.6, 4.6), 26.0),
        "infeed_a": Shot((ax - 13.0, ay - 3.0, 2.3), (ax - 5.5, ay, 0.9), 22.0, eye_end=(ax - 12.0, ay - 3.0, 2.2)),
        "outfeed_a": Shot((ax + 9.5, ay + 0.6, 2.6), (ax + 4.5, ay - 0.8, 1.0), 20.0),
        "service_a": Shot((ax + 5.0, ay - 7.5, 2.4), (ax - 0.5, ay - 3.25, 1.2), 22.0),
        "dock": Shot((-3.5, -3.8, 4.2), (0.3, -12.4, 1.2), 18.0),
    }
