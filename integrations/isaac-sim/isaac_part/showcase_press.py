"""A straight-side mechanical press as geometry: four columns on a bed with tie rods, a crown carrying the drive, a
slide with its die, guards and services. Plain arrays (no pxr). Local frame: x along the material flow, y across it,
z up, the floor at 0. Sizes follow large presses of this tonnage; only the plant's position is shared with the
physics, whose own bodies stay hidden. There are four groups, each moved as one prim: `base` never moves, `frame`
shakes, `slide` follows the ram, and `flywheel` spins about its own centre."""
from dataclasses import dataclass

import numpy as np

from .showcase_mesh import Merged

BED_TOP = 1.1
DIE_TOP = 1.55  # the blank lies here
COLUMN_TOP = 5.2
CROWN_TOP = 6.9
OPENING_Y = 1.4  # half the clear width between the columns, where the arms reach through
VISUAL_STROKE = 1.05  # the slide's travel on screen, at scale 1
SHUT_GAP = 0.03
HMI_AT = (-2.35, -1.6, 1.42)  # in front of where the plant's operator stands (2.0 and 2.1 m off the press centre)


@dataclass(frozen=True)
class PressGeometry:
    groups: dict  # group -> material -> Merged
    scale: float
    flywheel_center: tuple
    beacon_at: tuple
    andon_at: tuple
    hmi_at: tuple
    slide_bdc_z: float  # the slide group's height offset at bottom dead centre (the group's meshes are built at 0)
    stroke: float


class _Parts:
    """The groups being filled, with every size scaled in plan and in height."""

    def __init__(self, s):
        self.s = s
        self.groups = {name: {} for name in ("base", "frame", "slide", "flywheel")}

    def on(self, group, material) -> Merged:
        return self.groups[group].setdefault(material, Merged())

    def box(self, group, material, c, size, bevel=0.025):
        s = self.s
        self.on(group, material).box((c[0] * s, c[1] * s, c[2] * s), (size[0] * s, size[1] * s, size[2] * s),
                                     bevel * s)

    def cyl(self, group, material, c, r, h, axis="Z", segments=24, bevel=0.012):
        s = self.s
        self.on(group, material).cylinder((c[0] * s, c[1] * s, c[2] * s), r * s, h * s, axis, segments, bevel * s)


def build(scale: float) -> PressGeometry:
    p = _Parts(scale)
    _feet_and_bed(p)
    _columns(p)
    _crown(p)
    _drive(p)
    _services(p)
    _operator_station(p)
    _guards(p)
    _slide(p)
    _flywheel(p)
    s = scale
    return PressGeometry(p.groups, s, (1.0 * s, 2.95 * s, 6.1 * s), (-1.78 * s, -1.6 * s, 4.3 * s),
                         (-1.75 * s, 1.85 * s, 3.55 * s), HMI_AT,
                         (DIE_TOP + SHUT_GAP) * s, VISUAL_STROKE * s)


def _feet_and_bed(p):
    p.box("base", "floorplate", (0, 0, 0.02), (4.6, 6.4, 0.04), 0.01)
    for x in (-1.3, 1.3):
        for y in (-2.0, 2.0):
            p.box("base", "dark", (x, y, 0.1), (0.7, 0.7, 0.12))
            p.cyl("base", "steel", (x + 0.25, y + 0.25, 0.18), 0.04, 0.05, segments=6)
    p.box("frame", "body", (0, 0, 0.62), (3.3, 4.7, 0.96), 0.05)
    for y in (-1.8, -0.9, 0.0, 0.9, 1.8):
        for x in (-1.68, 1.68):
            p.box("frame", "body", (x, y, 0.6), (0.08, 0.14, 0.9))
    p.box("frame", "steel", (0, 0, BED_TOP + 0.08), (2.7, 3.4, 0.16), 0.01)
    for y in (-1.2, -0.6, 0.0, 0.6, 1.2):
        p.box("frame", "dark", (0, y, BED_TOP + 0.163), (2.7, 0.05, 0.01), 0.0)
    p.box("frame", "die", (0, 0, BED_TOP + 0.3), (1.9, 2.3, 0.3), 0.03)
    for x, y in ((-0.82, -1.0), (-0.82, 1.0), (0.82, -1.0), (0.82, 1.0)):
        p.cyl("frame", "steel", (x, y, DIE_TOP + 0.2), 0.055, 0.5, segments=14)
    for x in (-1.15, 1.15):
        for y in (-1.25, 1.25):
            p.box("frame", "trim", (x, y, BED_TOP + 0.23), (0.28, 0.2, 0.14))
    p.box("frame", "dark", (0, 0, DIE_TOP + 0.002), (1.45, 1.1, 0.005), 0.0)


def _columns(p):
    h = COLUMN_TOP - BED_TOP
    z = BED_TOP + h / 2
    for x in (-1.25, 1.25):
        for y in (-1.85, 1.85):
            p.box("frame", "body", (x, y, z), (0.95, 0.95, h), 0.06)
            sy = 1 if y > 0 else -1
            p.box("frame", "steel", (x, y - sy * 0.49, z - 0.3), (0.5, 0.04, h - 1.1), 0.005)
            sx = 1 if x > 0 else -1
            p.cyl("frame", "steel", (x + sx * 0.55, y + sy * 0.55, z + 0.3), 0.075, h + 1.8, segments=16)
            p.box("frame", "trim", (x + sx * 0.48, y, BED_TOP + 0.25), (0.015, 0.95, 0.2), 0.0)
            for side in (-1, 1):
                p.box("frame", "body", (x + sx * 0.49, y + side * 0.28, z + 0.4), (0.05, 0.1, h * 0.75))


def _crown(p):
    h = CROWN_TOP - COLUMN_TOP
    z = COLUMN_TOP + h / 2
    p.box("frame", "body", (0, 0, z), (3.7, 5.1, h), 0.07)
    for band_z in (COLUMN_TOP + 0.15, CROWN_TOP - 0.2):
        p.box("frame", "body", (0, 0, band_z), (3.84, 5.24, 0.16), 0.03)
    for x in (-1.0, 0.0, 1.0):
        for sy in (-1, 1):
            p.box("frame", "body", (x, sy * 2.58, z), (0.14, 0.08, h - 0.3))
    for y in (-1.7, -0.6, 0.6, 1.7):
        for sx in (-1, 1):
            p.box("frame", "body", (sx * 1.88, y, z), (0.08, 0.14, h - 0.3))
    for x in (-1.8, 1.8):
        for y in (-2.4, 2.4):
            p.cyl("frame", "dark", (x, y, CROWN_TOP + 0.08), 0.14, 0.16, segments=6)
    # top deck with handrails
    p.box("frame", "dark", (0, 0, CROWN_TOP + 0.02), (3.5, 4.9, 0.04), 0.0)
    for x in (-1.72, 1.72):
        p.cyl("frame", "trim", (x, 0, CROWN_TOP + 1.05), 0.022, 4.9, "Y", 8)
        for y in (-2.4, -1.2, 0.0, 1.2, 2.4):
            p.cyl("frame", "trim", (x, y, CROWN_TOP + 0.55), 0.02, 1.05, segments=8)
    for y in (-2.42, 2.42):
        p.cyl("frame", "trim", (0, y, CROWN_TOP + 1.05), 0.022, 3.45, "X", 8)


def _drive(p):
    top = CROWN_TOP + 0.05
    p.box("frame", "dark", (1.0, 1.7, top + 0.08), (1.0, 1.3, 0.16))
    p.cyl("frame", "motor", (1.0, 1.65, top + 0.58), 0.42, 1.15, "Y", 28, 0.03)
    for y in (1.2, 1.4, 1.6, 1.8, 2.0):
        p.cyl("frame", "motor", (1.0, y, top + 0.58), 0.46, 0.05, "Y", 28)
    p.cyl("frame", "steel", (1.0, 2.35, top + 0.58), 0.13, 0.3, "Y", 16)
    p.box("frame", "steel", (1.0, 1.05, top + 0.85), (0.3, 0.2, 0.25))
    # the belt guard from the motor down to the flywheel
    p.box("frame", "trim", (1.0, 2.45, 6.95), (0.5, 0.26, 1.7), 0.03)
    for x, z_ in ((-0.6, top + 0.5), (-1.2, top + 0.5)):
        p.cyl("frame", "steel", (x, -1.3, z_), 0.27, 1.0, segments=20, bevel=0.06)
    p.box("frame", "cabinet", (-0.8, 0.9, top + 0.35), (1.1, 0.7, 0.7))
    p.box("frame", "trim", (-0.8, 0.54, top + 0.5), (0.5, 0.02, 0.05), 0.0)


def _services(p):
    """Grease lines, conduit and the drive cabinet that stands beside the press."""
    for i, y in enumerate((-2.42, -2.36, -2.30)):
        p.cyl("frame", "steel" if i else "copper", (-1.8, y, 3.2), 0.016, 3.8, segments=8)
    p.box("frame", "dark", (-1.82, -2.36, 5.02), (0.1, 0.3, 0.1), 0.01)
    p.box("frame", "dark", (-1.82, -2.36, 1.42), (0.1, 0.3, 0.1), 0.01)
    p.box("frame", "cabinet", (-1.82, -1.85, 2.3), (0.3, 0.7, 1.0), 0.015)  # lube pump
    p.box("frame", "steel", (-1.98, -1.85, 2.45), (0.01, 0.42, 0.22), 0.0)
    # the drive cabinet stands beside the outfeed, clear of the aisle the technicians walk
    p.box("base", "cabinet", (5.0, -2.25, 1.0), (1.8, 0.45, 2.0), 0.015)
    for i in range(3):
        p.box("base", "cabinet", (5.0 + (i - 1) * 0.6, -2.48, 1.0), (0.56, 0.02, 1.85), 0.004)
        p.box("base", "steel", (5.0 + (i - 1) * 0.6 + 0.2, -2.5, 1.1), (0.03, 0.03, 0.2), 0.005)
    p.box("base", "dark", (5.0, -2.25, 2.06), (1.7, 0.3, 0.12), 0.01)


def _operator_station(p):
    """The post the HMI stands on, not scaled: the operator's place is the same at every press."""
    x, y, z = HMI_AT
    p.on("base", "dark").cylinder((x, y + 0.08, z / 2), 0.04, z, "Z", 10)
    p.on("base", "dark").box((x, y + 0.08, 0.02), (0.4, 0.4, 0.04), 0.005)


def _guards(p):
    """Fixed guards across the side gaps, and light-curtain posts on the arms' two sides."""
    z0, z1 = BED_TOP + 0.15, 3.9
    for sy in (-1, 1):
        y = sy * 2.35
        for x in np.arange(-0.72, 0.73, 0.08):  # welded wire mesh: the die area stays visible through it
            p.box("frame", "mesh", (x, y, (z0 + z1) / 2), (0.012, 0.012, z1 - z0), 0.0)
        for z in np.arange(z0 + 0.04, z1, 0.08):
            p.box("frame", "mesh", (0, y, z), (1.5, 0.012, 0.012), 0.0)
        for x in (-0.76, 0.76):
            p.box("frame", "trim", (x, y, (z0 + z1) / 2), (0.05, 0.05, z1 - z0), 0.008)
        for z in (z0, (z0 + z1) / 2, z1):
            p.box("frame", "trim", (0, y, z), (1.55, 0.05, 0.05), 0.008)
    for sx in (-1, 1):
        for y in (-1.5, 1.5):
            p.box("base", "trim", (sx * 1.95, y, 1.1), (0.07, 0.07, 2.2), 0.01)
            p.box("base", "dark", (sx * 1.95, y, 2.24), (0.09, 0.09, 0.09), 0.01)


def _slide(p):
    """Built with its zero where the upper die shuts on the lower one; the visuals add height above it."""
    zero = DIE_TOP + SHUT_GAP
    p.box("slide", "die", (0, 0, zero + 0.15), (1.9, 2.3, 0.3), 0.03)
    for x, y in ((-0.82, -1.0), (-0.82, 1.0), (0.82, -1.0), (0.82, 1.0)):
        p.cyl("slide", "steel", (x, y, zero + 0.02), 0.075, 0.06, segments=14)
    b = zero + 0.3
    p.box("slide", "body", (0, 0, b + 0.45), (2.15, 2.7, 0.9), 0.04)
    p.box("slide", "steel", (0, 0, b + 0.03), (2.2, 2.75, 0.06), 0.008)
    for y in (-1.1, -0.55, 0.0, 0.55, 1.1):
        for sx in (-1, 1):
            p.box("slide", "body", (sx * 1.09, y, b + 0.5), (0.06, 0.12, 0.78))
    for sy in (-1, 1):
        p.box("slide", "steel", (0, sy * 1.37, b + 0.45), (0.5, 0.04, 0.8), 0.005)
    for x in (-1.0, 1.0):
        p.box("slide", "trim", (x, 0, b + 0.14), (0.25, 2.5, 0.08), 0.008)
    for y in (-0.8, 0.8):
        p.cyl("slide", "steel", (0, y, b + 0.9 + 1.3), 0.17, 2.6, segments=20)


def _flywheel(p):
    """Built around its own centre (0, 0, 0): the visuals spin it about its Y axis."""
    p.cyl("flywheel", "dark", (0, 0, 0), 0.95, 0.3, "Y", 40, 0.02)
    p.cyl("flywheel", "steel", (0, -0.17, 0), 0.8, 0.04, "Y", 40)
    p.cyl("flywheel", "dark", (0, -0.2, 0), 0.2, 0.1, "Y", 20)
    p.box("flywheel", "dark", (0, -0.21, 0), (1.5, 0.04, 0.09), 0.0)
    p.box("flywheel", "dark", (0, -0.21, 0), (0.09, 0.04, 1.5), 0.0)
    p.box("flywheel", "trim", (0.0, -0.22, 0.6), (0.12, 0.02, 0.32), 0.0)
