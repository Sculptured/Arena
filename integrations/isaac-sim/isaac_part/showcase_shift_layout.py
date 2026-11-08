"""Where the shift's scene pieces stand in the showcase, plain arithmetic: robot welding cell R1 east of Press Line B,
two AMRs on their rounds, the energy board, the IT/OT gateway's cabinet and the dock's arrivals board; the welding arm's
cycle; and the camera's shots of them. Only seen: nothing here reaches the plant."""
from dataclasses import dataclass

import numpy as np

from . import showcase_motion as motion
from . import showcase_robot as robot
from .showcase_camera import Shot

CELL = (12.5, 6.5)  # the cell's centre, east of Line B's stillage and the technicians' aisle (x 7.4)
CELL_SIZE = (5.2, 4.4)  # fenced floor, x by y
ARM_BASE = (CELL[0] - 1.0, CELL[1], 0.62)  # on a pedestal, the table east of it
TABLE = (CELL[0] + 0.55, CELL[1], 0.86)  # the welding fixture's top
TORCH = 0.32  # from the arm's last joint to the torch tip, along the tool's down axis
WELD_CYCLE_S = 14.0
PARK_S = 2.5  # how long the arm takes to park after a safe stop
UR_JOINTS = ("shoulder_pan_joint", "shoulder_lift_joint", "elbow_joint", "wrist_1_joint", "wrist_2_joint",
             "wrist_3_joint")
UR_LINKS = ("shoulder_link", "upper_arm_link", "forearm_link", "wrist_1_link", "wrist_2_link", "wrist_3_link")
ENERGY_BOARD = (10.4, -1.2, 2.7)  # on the east wall, facing west
GATEWAY = (10.6, -4.6)  # the network cabinet, its door facing west
DOCK_BOARD = (-4.2, -8.9, 2.5)  # facing north, over the receiving dock
SCREEN_WEST, SCREEN_NORTH, SCREEN_SOUTH = -90.0, 180.0, 0.0  # a screen's yaw: which way it faces

# (phase in the weld cycle, seam point offset from the table top centre in metres, welding)
WELD_PATH = ((0.0, (-0.05, -0.55, 0.30), False), (0.12, (-0.05, -0.45, 0.02), False),
             (0.42, (-0.05, 0.45, 0.02), True), (0.50, (-0.05, 0.45, 0.25), False),
             (0.58, (0.25, 0.40, 0.25), False), (0.64, (0.25, 0.40, 0.02), False),
             (0.88, (0.25, -0.40, 0.02), True), (0.95, (0.25, -0.50, 0.30), False))


@dataclass(frozen=True)
class WeldCycle:
    motion: robot.CycleMotion
    welding: tuple  # (phase from, phase to) ranges in which the torch burns
    park: np.ndarray


def _torch_down() -> np.ndarray:
    """The tool frame pointing its +X along the seam and its -Z (the torch) down."""
    return np.array([[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]])


def weld_cycle(chain) -> WeldCycle:
    """Joint keys for the seams, solved from the arm's own joints, each from the last so the arm keeps one branch."""
    q = np.array([0.0, -1.2, 1.6, -1.95, -1.57, 0.0])
    keys, burning = [], []
    for phase, (dx, dy, dz), welding in WELD_PATH:
        at = (TABLE[0] + dx - ARM_BASE[0], TABLE[1] + dy - ARM_BASE[1], TABLE[2] + dz - ARM_BASE[2])
        q = robot.solve(chain, robot.pose(at, _torch_down()), q)
        keys.append((phase, q.copy()))
        if welding:
            burning.append(phase)
    phases = [p for p, *_ in WELD_PATH]
    ranges = tuple((phases[phases.index(p) - 1], p) for p in burning)
    park = robot.solve(chain, robot.pose((0.35, 0.0, 0.85), _torch_down()), q)
    return WeldCycle(robot.CycleMotion(keys), ranges, park)


def burning(cycle: WeldCycle, phase: float) -> bool:
    p = phase % 1.0
    return any(a <= p < b for a, b in cycle.welding)


def amr_rounds() -> list:
    """Two AMRs: one takes welded brackets from the cell to the dock, the other brings blank racks along the line heads."""
    leg = motion.Leg
    cx, cy = CELL
    dock = leg((9.0, -7.2), (-1.8, -7.2), 14.0)
    to_cell = [leg((-1.8, -7.2), (9.0, -7.2), 14.0), leg((9.0, -7.2, 0.0), (9.0, -7.2, 90.0), 2.0),
               leg((9.0, -7.2), (9.0, cy - 1.4), 15.0), leg((9.0, cy - 1.4, 90.0), (9.0, cy - 1.4, 0.0), 2.0),
               leg((9.0, cy - 1.4), (cx - 2.9, cy - 1.4), 3.0), leg((cx - 2.9, cy - 1.4), (cx - 2.9, cy - 1.4), 6.0),
               leg((cx - 2.9, cy - 1.4), (9.0, cy - 1.4), 3.0, reverse=True),
               leg((9.0, cy - 1.4, 0.0), (9.0, cy - 1.4, -90.0), 2.0), leg((9.0, cy - 1.4), (9.0, -7.2), 15.0),
               leg((9.0, -7.2, -90.0), (9.0, -7.2, 180.0), 2.0), dock,
               leg((-1.8, -7.2), (-1.8, -7.2), 5.0)]
    heads = [leg((-16.5, -3.0), (-16.5, 13.5), 20.0), leg((-16.5, 13.5), (-16.5, 13.5), 5.0),
             leg((-16.5, 13.5, 90.0), (-16.5, 13.5, -90.0), 3.0), leg((-16.5, 13.5), (-16.5, -3.0), 20.0),
             leg((-16.5, -3.0), (-16.5, -3.0), 5.0), leg((-16.5, -3.0, -90.0), (-16.5, -3.0, 90.0), 3.0)]
    return [motion.Round(to_cell), motion.Round(heads)]


def shots() -> dict:
    """The camera's shots of the shift's pieces."""
    cx, cy = CELL
    ex, ey, ez = ENERGY_BOARD
    gx, gy = GATEWAY
    dx, dy, dz = DOCK_BOARD
    return {
        "cell_r1": Shot((cx - 7.5, cy - 6.0, 3.4), (cx, cy, 1.0), 20.0, eye_end=(cx - 6.5, cy - 6.4, 3.1)),
        "cell_r1_close": Shot((cx - 2.6, cy - 2.6, 1.7), (TABLE[0], TABLE[1], TABLE[2] + 0.2), 24.0,
                              eye_end=(cx - 2.2, cy - 2.8, 1.6)),
        "energy_board": Shot((ex - 5.5, ey - 1.6, 2.0), (ex, ey, ez), 26.0, eye_end=(ex - 5.0, ey - 1.2, 2.0)),
        "gateway": Shot((gx - 4.2, gy - 2.2, 1.7), (gx, gy, 1.2), 24.0, eye_end=(gx - 3.8, gy - 1.8, 1.7)),
        "dock_board": Shot((dx + 2.0, dy + 5.5, 2.0), (dx, dy, dz), 24.0, eye_end=(dx + 1.4, dy + 5.2, 2.0)),
        "amr": Shot((4.0, -11.0, 2.4), (0, 0, 0.5), 22.0, follow="amr"),
        "wide_shift": Shot((19.0, -11.0, 7.5), (3.0, 4.0, 1.2), 16.0, eye_end=(18.0, -12.0, 7.2)),
    }


def screen_widths() -> dict:
    return {"energy": 2.4, "gateway": 0.5, "dock": 1.8, "cell": 0.5}
