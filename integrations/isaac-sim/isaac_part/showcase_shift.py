"""The shift's scene pieces in the showcase, built with the showcase's Dresser from NVIDIA's stock assets: robot
welding cell R1 (a UR10e on a pedestal, its welding fixture, fence, stack light and HMI), two Nova Carter AMRs
carrying KLT bins, the energy board, the IT/OT gateway's cabinet with its stack light and screen, and the dock's
arrivals board. As with the rest of the showcase, nothing here is simulated: physics is stripped, the visuals move
it by hand."""
from dataclasses import dataclass, field

import numpy as np

from . import showcase_mesh as mesh
from . import showcase_robot as robot
from . import showcase_shift_layout as plan

ROOT = "/World/Showcase/Shift"
ASSETS = {
    "ur10e": "/Isaac/6.2/Isaac/Robots/UniversalRobots/ur10e/ur10e.usd",
    "nova_carter": "/Isaac/6.2/Isaac/Robots/NVIDIA/NovaCarter/nova_carter.usd",
    "klt": "/Isaac/6.2/Isaac/Props/KLT_Bin/small_KLT.usd",
}
AMR_DECK = 0.36  # where the bins sit on the Nova Carter's deck


@dataclass
class WeldArm:
    links: dict  # link name -> prim path
    chain: robot.Chain
    cycle: plan.WeldCycle
    glow: str  # the lit spark at the torch tip (shown while welding)
    spark: str  # its light


@dataclass
class ShiftDressed:
    arm: WeldArm
    cell_lamps: dict
    cell_beacon: dict
    cell_screen: str
    amrs: list = field(default_factory=list)  # (path, Round)
    energy_screen: str = ""
    gateway_screen: str = ""
    gateway_lamps: dict = field(default_factory=dict)
    dock_screen: str = ""


def dress(d) -> ShiftDressed:
    """Adds the pieces to the stage the Dresser `d` holds (before physics starts)."""
    from . import showcase_scene
    showcase_scene.ASSETS.update(ASSETS)
    d.b.xform(ROOT)
    arm = _cell(d)
    lamps, beacon = d.lamps(f"{ROOT}/Cell/Lamps", (plan.CELL[0] - 2.4, plan.CELL[1] - 2.0, 1.6), "CELL_R1")
    cx, cy = plan.CELL
    widths = plan.screen_widths()
    cell_screen = d.screen(f"{ROOT}/Cell/Hmi", (cx - 1.6, cy - plan.CELL_SIZE[1] / 2 - 0.1, 1.5), widths["cell"],
                           plan.SCREEN_SOUTH, "cell_r1")
    out = ShiftDressed(arm, lamps, beacon, cell_screen)
    out.amrs = _amrs(d)
    out.energy_screen = _energy_board(d, widths["energy"])
    out.gateway_screen, out.gateway_lamps = _gateway(d, widths["gateway"])
    out.dock_screen = _dock_board(d, widths["dock"])
    _lights(d)
    return out


def _lights(d) -> None:
    """High-bay fixtures over the cell, the boards and the gateway, like those over the lines."""
    Gf = d.b.Gf
    for i, (x, y) in enumerate(((plan.CELL[0], plan.CELL[1]), (9.5, -2.5), (-3.0, -7.5))):
        rect = d.UsdLux.RectLight.Define(d.stage, f"{ROOT}/Bay_{i}")
        rect.CreateWidthAttr(5.0)
        rect.CreateHeightAttr(2.0)
        rect.CreateIntensityAttr(30000.0)
        rect.CreateColorTemperatureAttr(4800.0)
        rect.CreateEnableColorTemperatureAttr(True)
        rect.AddTranslateOp().Set(Gf.Vec3d(x, y, 8.8))


# ---- robot cell R1 -------------------------------------------------------------------------------------------

def _cell(d) -> WeldArm:
    root = f"{ROOT}/Cell"
    d.b.xform(root)
    _fence(d, root)
    _table(d, root)
    base = plan.ARM_BASE
    joints = d.asset(f"{root}/Arm", "ur10e", base, before_strip=lambda prim: _joints(d, prim))
    chain = robot.Chain(np.eye(4), joints, robot.pose((0.0, 0.0, plan.TORCH)))
    links = {name: f"{root}/Arm/Asset/{name}" for name in plan.UR_LINKS}
    for link in links.values():
        xf = d.UsdGeom.Xformable(d.stage.GetPrimAtPath(link))
        xf.ClearXformOpOrder()
        xf.AddTransformOp()
    d.pedestal(f"{root}/Arm/Pedestal", base[2])
    glow, spark = _torch(d, links["wrist_3_link"])
    return WeldArm(links, chain, plan.weld_cycle(chain), glow, spark)


def _joints(d, prim) -> list:
    """The UR10e's six revolute joints, as its asset declares them (switched off right after)."""
    from pxr import Usd
    phys = d.UsdPhysics
    found = {}
    for p in Usd.PrimRange(prim):
        if p.IsA(phys.RevoluteJoint):
            j = phys.RevoluteJoint(p)
            found[p.GetName()] = robot.joint_from_usd(
                j.GetBody0Rel().GetTargets()[0].name, j.GetBody1Rel().GetTargets()[0].name,
                tuple(j.GetLocalPos0Attr().Get()), _wxyz(j.GetLocalRot0Attr().Get()),
                tuple(j.GetLocalPos1Attr().Get()), _wxyz(j.GetLocalRot1Attr().Get()), j.GetAxisAttr().Get(),
                j.GetLowerLimitAttr().Get(), j.GetUpperLimitAttr().Get())
    return [found[name] for name in plan.UR_JOINTS]


def _wxyz(q) -> tuple:
    im = q.GetImaginary()
    return (q.GetReal(), im[0], im[1], im[2])


def _fence(d, root) -> None:
    """Yellow posts and dark mesh panels round the cell, open on the west side where the AMR docks."""
    cx, cy = plan.CELL
    w, h = plan.CELL_SIZE
    posts, panels, plate = mesh.Merged(), mesh.Merged(), mesh.Merged()
    plate.box((cx, cy, 0.005), (w, h, 0.01), 0.0)
    corners = [(cx - w / 2, cy - h / 2), (cx + w / 2, cy - h / 2), (cx + w / 2, cy + h / 2), (cx - w / 2, cy + h / 2)]
    for (x0, y0), (x1, y1) in zip(corners, corners[1:] + corners[:1]):
        if x0 == x1 == cx - w / 2:  # the west side stays open
            continue
        n = max(1, int(round(max(abs(x1 - x0), abs(y1 - y0)) / 1.3)))
        for i in range(n + 1):
            f = i / n
            posts.box((x0 + (x1 - x0) * f, y0 + (y1 - y0) * f, 1.0), (0.06, 0.06, 2.0), 0.005)
        size = (abs(x1 - x0) or 0.02, abs(y1 - y0) or 0.02, 1.7)
        panels.box(((x0 + x1) / 2, (y0 + y1) / 2, 1.05), size, 0.0)
    d.mesh(f"{root}/Posts", posts.arrays(), d.paint("trim"))
    d.mesh(f"{root}/Panels", panels.arrays(), d.paint("mesh"))
    d.mesh(f"{root}/Plate", plate.arrays(), d.paint("floorplate"))


def _table(d, root) -> None:
    """The welding fixture: a steel table with clamps and a bracket on it."""
    x, y, top = plan.TABLE
    frame, part = mesh.Merged(), mesh.Merged()
    frame.box((x, y, top - 0.03), (1.0, 1.3, 0.06), 0.005)
    for sx in (-1, 1):
        for sy in (-1, 1):
            frame.box((x + sx * 0.42, y + sy * 0.57, (top - 0.06) / 2), (0.08, 0.08, top - 0.06), 0.005)
        frame.box((x + sx * 0.32, y, top + 0.05), (0.06, 1.1, 0.06), 0.004)  # the clamp rails
    part.box((x - 0.3, y, top + 0.03), (0.12, 1.0, 0.06), 0.004)
    part.box((x, y, top + 0.03), (0.12, 0.9, 0.06), 0.004)
    part.box((x - 0.15, y, top + 0.005), (0.5, 1.05, 0.01), 0.002)
    d.mesh(f"{root}/Table", frame.arrays(), d.paint("dark"))
    d.mesh(f"{root}/Workpiece", part.arrays(), d.paint("formed"))


def _torch(d, wrist) -> tuple:
    """The torch along the flange's axis, and at its tip a spark (a lit bead and a light) the visuals switch."""
    gun, nozzle = mesh.Merged(), mesh.Merged()
    gun.cylinder((0, 0, 0.08), 0.045, 0.16, "Z", 16)
    nozzle.cylinder((0, 0, 0.16 + (plan.TORCH - 0.16) / 2), 0.018, plan.TORCH - 0.16, "Z", 12)
    d.mesh(f"{wrist}/TorchBody", gun.arrays(), d.paint("motor"))
    d.mesh(f"{wrist}/TorchNozzle", nozzle.arrays(), d.paint("copper"))
    rgb = (1.0, 0.75, 0.35)
    glow = d._lens(f"{wrist}/Spark", (0, 0, plan.TORCH + 0.005), 0.012, 0.02, rgb, "cell_spark")
    light = d.UsdLux.SphereLight.Define(d.stage, f"{wrist}/SparkLight")
    light.CreateRadiusAttr(0.02)
    light.CreateIntensityAttr(0.0)
    light.CreateColorAttr(d.b.Gf.Vec3f(0.75, 0.85, 1.0))
    light.AddTranslateOp().Set(d.b.Gf.Vec3d(0.0, 0.0, plan.TORCH + 0.02))
    return glow, f"{wrist}/SparkLight"


# ---- around the plant ----------------------------------------------------------------------------------------

def _amrs(d) -> list:
    out = []
    for i, route in enumerate(plan.amr_rounds()):
        path = f"{ROOT}/Amr{i}"
        d.b.xform(path)
        d.asset(f"{path}/Body", "nova_carter", (0, 0, 0), 0.0)
        for j, dx in enumerate((-0.17, 0.17)):
            d.asset(f"{path}/Bin{j}", "klt", (dx, 0.0, AMR_DECK), 90.0)
        out.append((path, route))
    return out


def _energy_board(d, width) -> str:
    x, y, z = plan.ENERGY_BOARD
    pole = mesh.Merged()
    pole.box((x + 0.1, y, z / 2), (0.08, 0.08, z), 0.004)
    d.mesh(f"{ROOT}/EnergyBoard_Pole", pole.arrays(), d.paint("dark"))
    return d.screen(f"{ROOT}/EnergyBoard", (x, y, z), width, plan.SCREEN_WEST, "energy_board")


def _gateway(d, width) -> tuple:
    x, y = plan.GATEWAY
    root = f"{ROOT}/Gateway"
    body, door, tray = mesh.Merged(), mesh.Merged(), mesh.Merged()
    body.box((x, y, 1.0), (0.6, 0.8, 2.0), 0.01)
    door.box((x - 0.305, y, 1.0), (0.01, 0.72, 1.85), 0.0)
    tray.box((x, y, 2.45), (0.2, 0.25, 0.9), 0.004)  # the cable riser to the ceiling tray
    d.b.xform(root)
    d.mesh(f"{root}/Body", body.arrays(), d.paint("cabinet"))
    d.mesh(f"{root}/Door", door.arrays(), d.paint("dark"))
    d.mesh(f"{root}/Riser", tray.arrays(), d.paint("steel"))
    lamps, _ = d.lamps(f"{root}/Lamps", (x - 0.15, y + 0.25, 2.0), "GW_1")
    screen = d.screen(f"{root}/Screen", (x - 0.32, y, 1.45), width, plan.SCREEN_WEST, "gateway")
    return screen, lamps


def _dock_board(d, width) -> str:
    x, y, z = plan.DOCK_BOARD
    pole = mesh.Merged()
    pole.box((x, y - 0.1, z / 2), (0.08, 0.08, z), 0.004)
    d.mesh(f"{ROOT}/DockBoard_Pole", pole.arrays(), d.paint("dark"))
    return d.screen(f"{ROOT}/DockBoard", (x, y, z), width, plan.SCREEN_NORTH, "dock_board")
