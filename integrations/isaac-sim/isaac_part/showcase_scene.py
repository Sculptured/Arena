"""Dresses the factory's stage as a believable stamping plant, using NVIDIA's free Isaac assets, referenced at run time
from NVIDIA's public server (nothing is copied into this repo). The physics the plant needs stays exactly as the
factory built it: its press articulations and iw.hub run untouched, only made invisible where the dressing replaces
them. Everything added here has its physics removed, so PhysX never sees it and the visuals move it by hand."""
import importlib
from dataclasses import dataclass, field

import numpy as np

from . import showcase_layout as plan
from . import showcase_mesh as mesh
from . import showcase_motion as motion
from . import showcase_press as press
from . import showcase_robot as robot

ROOT = "/World/Showcase"
# Under the server's Assets folder. All of these answered on 2026-11-05.
ASSETS = {
    "hall": "/ArchVis/Industrial/Buildings/Warehouse/Warehouse01.usd",
    "arm": "/Isaac/6.2/Isaac/Robots/Fanuc/r2000ic_165f/r2000ic_165f.usd",
    "conveyor": "/Isaac/6.2/Isaac/Props/Conveyors/ConveyorBelt_A08.usd",
    "forklift": "/Isaac/6.2/Isaac/Props/Forklift/forklift.usd",
    "pallet": "/Isaac/6.2/Isaac/Props/Pallet/pallet.usd",
    "rack": "/ArchVis/Industrial/Racks/RackLong_A1.usd",
    "rack_large": "/ArchVis/Industrial/Racks/RackLarge_A1.usd",
}
# The hall's columns stand on a 6.7 m x 15 m grid. This puts a row of them in the aisle between lines C and B (y 3.25),
# clear of the route the technicians walk at x 7.4, and the next rows south of the dock and north of line A.
HALL_AT = (-0.65, 3.75, 0.0)
HALL_SCALE = (0.01, 0.01, 0.01)  # the hall is authored in centimetres
FORKLIFT_YAW = -90.0  # the asset is long along Y; this turns its forks to the wrapper's +X
RACK_ROW_X = 20.0  # stock racks this far west of the presses, beyond the forklift's lane
HIDDEN_NAMES = ("Label", "Sign", "Nameplate")
PAINTS = {  # name: (colour, roughness, metallic)
    "body": ((0.26, 0.37, 0.46), 0.45, 0.25),
    "trim": ((0.85, 0.62, 0.05), 0.5, 0.0),
    "steel": ((0.62, 0.63, 0.64), 0.32, 1.0),
    "die": ((0.30, 0.31, 0.32), 0.55, 0.7),
    "dark": ((0.035, 0.037, 0.04), 0.65, 0.2),
    "motor": ((0.10, 0.20, 0.30), 0.45, 0.3),
    "cabinet": ((0.50, 0.52, 0.50), 0.55, 0.1),
    "copper": ((0.72, 0.42, 0.22), 0.35, 1.0),
    "mesh": ((0.05, 0.05, 0.05), 0.6, 0.3),
    "floorplate": ((0.20, 0.20, 0.20), 0.7, 0.6),
    "blank": ((0.80, 0.81, 0.83), 0.42, 0.85),
    "formed": ((0.66, 0.67, 0.69), 0.36, 1.0),
    "stillage": ((0.10, 0.22, 0.42), 0.55, 0.2),
    "tape": ((0.9, 0.68, 0.05), 0.65, 0.0),
    "aisle": ((0.82, 0.82, 0.80), 0.7, 0.0),
    "rubber": ((0.02, 0.02, 0.02), 0.85, 0.0),
}
DOME = 1200.0
BAY_LIGHT = 30000.0  # high-bay fixtures over each line, 5 m x 2 m
LIT = 700.0  # emissive intensity of a lit lens; far brighter washes out to pastel through the tonemapper
POOL_BLANKS = 9
POOL_FORMED = 10
STILLAGE_PARTS = 24
PARKED_Z = -6.0  # parts not in use wait under the floor


@dataclass
class Arm:
    links: dict  # link name -> prim path
    chain: robot.Chain
    cycle: robot.CycleMotion
    base: tuple


@dataclass
class DressedLine:
    plan: plan.LineLayout
    geometry: press.PressGeometry
    frame: str
    slide: str
    flywheel: str
    loader: Arm
    unloader: Arm
    flow: motion.LineFlow
    blanks: list
    formed: list
    stacked: list
    lamps: dict  # colour -> shader path
    beacon: dict
    screens: list  # shader paths of its andon board and HMI


@dataclass
class Dressed:
    lines: list = field(default_factory=list)
    forklift: str = ""
    fork_load: str = ""
    forklift_round: motion.Round | None = None
    shots: dict = field(default_factory=dict)
    press_hz: dict = field(default_factory=dict)


class Dresser:
    def __init__(self, b, assets_root, data, log):
        self.b, self.log = b, log
        self.stage = b.stage
        self.server = assets_root.rsplit("/Isaac/", 1)[0]  # .../Assets
        self.data = data
        self.layout = importlib.import_module("layout")
        from pxr import Sdf, UsdGeom, UsdLux, UsdPhysics, Vt
        self.Sdf, self.UsdGeom, self.UsdLux, self.UsdPhysics, self.Vt = Sdf, UsdGeom, UsdLux, UsdPhysics, Vt
        self.paints = {}

    # ---- what the factory built ------------------------------------------------------------------------------

    def retire_old_visuals(self) -> None:
        """Hides what the showcase replaces. Old shells and conveyors go inactive: their colliders and kinematic
        rollers are not the plant's, and PhysX steps faster without them."""
        layout, stage = self.layout, self.stage
        for path in ("/World/Plant2/Shell/Asset", "/World/Plant1/Shell/Asset", "/World/Lights", "/World/Plant1/Shop"):
            stage.GetPrimAtPath(path).SetActive(False) if stage.GetPrimAtPath(path) else None
        for lid in layout.LINES:
            root = layout.line_path(lid)
            for part in ("ConveyorIn", "ConveyorOut"):
                stage.GetPrimAtPath(f"{root}/{part}").SetActive(False)
            for part in ("Parts", "BlankPallet", "Bin", "Zone"):
                self.UsdGeom.Imageable(stage.GetPrimAtPath(f"{root}/{part}")).MakeInvisible()
        for mid in layout.MACHINES:
            self.UsdGeom.Imageable(stage.GetPrimAtPath(layout.PRIMS[mid]["press"])).MakeInvisible()
        for name in ("Zone", "Leveler", "Jamb_L", "Jamb_R", "Bumper_L", "Bumper_R"):
            prim = stage.GetPrimAtPath(f"/World/Plant2/Dock/{name}")
            if prim:
                self.UsdGeom.Imageable(prim).MakeInvisible()
        for prim in stage.Traverse():
            name = prim.GetName()
            if name in HIDDEN_NAMES or name.endswith("_Label"):
                self.UsdGeom.Imageable(prim).MakeInvisible()

    # ---- building blocks --------------------------------------------------------------------------------------

    def paint(self, name):
        if name not in self.paints:
            color, rough, metal = PAINTS[name]
            self.paints[name] = self.b.material(f"Show_{name}", color, roughness=rough, metallic=metal)
        return self.paints[name]

    def mesh(self, path, arrays, material):
        UsdGeom, Vt = self.UsdGeom, self.Vt
        m = UsdGeom.Mesh.Define(self.stage, path)
        points = np.asarray(arrays["points"], dtype=np.float32)
        m.CreatePointsAttr(Vt.Vec3fArray.FromNumpy(points))
        m.CreateNormalsAttr(Vt.Vec3fArray.FromNumpy(np.asarray(arrays["normals"], dtype=np.float32)))
        m.SetNormalsInterpolation("vertex")
        m.CreateFaceVertexCountsAttr(Vt.IntArray.FromNumpy(np.asarray(arrays["counts"], dtype=np.int32)))
        m.CreateFaceVertexIndicesAttr(Vt.IntArray.FromNumpy(np.asarray(arrays["indices"], dtype=np.int32)))
        m.CreateSubdivisionSchemeAttr("none")
        m.CreateExtentAttr(Vt.Vec3fArray.FromNumpy(np.array([points.min(axis=0), points.max(axis=0)], dtype=np.float32)))
        if "uvs" in arrays:
            st = UsdGeom.PrimvarsAPI(m).CreatePrimvar("st", self.Sdf.ValueTypeNames.TexCoord2fArray,
                                                      UsdGeom.Tokens.vertex)
            st.Set(Vt.Vec2fArray.FromNumpy(np.asarray(arrays["uvs"], dtype=np.float32)))
        self.b.bind(m.GetPrim(), material)
        return m

    def merged(self, root, group: dict) -> None:
        for material, parts in group.items():
            self.mesh(f"{root}/{material}", parts.arrays(), self.paint(material))

    def asset(self, path, key, at=(0, 0, 0), yaw=0.0, scale=None, before_strip=None):
        """A referenced asset, stripped of physics, under a wrapper this module can move."""
        self.b.xform(path, at, yaw, s=scale)
        prim = self.stage.DefinePrim(f"{path}/Asset")
        prim.GetReferences().AddReference(self.server + ASSETS[key])
        found = before_strip(prim) if before_strip else None
        self._strip_physics(prim)
        return found if before_strip else prim

    def _strip_physics(self, root) -> None:
        """Nothing under `root` is simulated: no bodies, joints or colliders, for PhysX to spend time on."""
        from pxr import Usd
        phys = self.UsdPhysics
        for prim in Usd.PrimRange(root):
            if prim.IsInstanceProxy():
                continue
            if prim.IsA(phys.Joint):
                prim.SetActive(False)
                continue
            for api in (phys.ArticulationRootAPI, phys.RigidBodyAPI, phys.MassAPI, phys.CollisionAPI,
                        phys.MeshCollisionAPI):
                if prim.HasAPI(api):
                    prim.RemoveAPI(api)
            if prim.GetName() == "collisions":
                prim.SetActive(False)

    # ---- the plant --------------------------------------------------------------------------------------------

    def hall(self) -> None:
        self.asset(f"{ROOT}/Hall", "hall", HALL_AT, 0.0, HALL_SCALE)

    def press_body(self, line) -> tuple:
        geometry = press.build(line.scale)
        root = f"{ROOT}/{line.line.replace('-', '_')}/Press"
        self.b.xform(root, (line.cx, line.cy, 0))
        paths = {}
        for group, parts in geometry.groups.items():
            paths[group] = f"{root}/{group}"
            at = geometry.flywheel_center if group == "flywheel" else (0, 0, 0)
            prim = self.b.xform(paths[group], at, ops=("translate",))
            if group == "flywheel":
                self.UsdGeom.Xformable(prim).AddRotateYOp().Set(0.0)
            self.merged(paths[group], parts)
        return geometry, paths

    def arm(self, path, base, make, line) -> Arm:
        joints = self.asset(path, "arm", base, before_strip=self._arm_joints)
        chain = robot.Chain(np.eye(4), joints).with_world_aligned_tool((plan.TOOL_BOOM, 0.0, -plan.TOOL_DROP))
        links = {name: f"{path}/Asset/{name}" for name in plan.ARM_LINKS}
        for link in links.values():
            xf = self.UsdGeom.Xformable(self.stage.GetPrimAtPath(link))
            xf.ClearXformOpOrder()
            xf.AddTransformOp()
        self._tool(f"{links['J6_link']}/Tool", chain)
        self.pedestal(f"{path}/Pedestal", base[2])
        return Arm(links, chain, make(chain, line), base)

    def _arm_joints(self, prim) -> list:
        """The arm's six joints as its asset declares them. Their prims are switched off right after this."""
        from pxr import Usd
        phys = self.UsdPhysics
        found = {}
        for p in Usd.PrimRange(prim):
            if p.IsA(phys.RevoluteJoint):
                j = phys.RevoluteJoint(p)
                found[p.GetName()] = robot.joint_from_usd(
                    j.GetBody0Rel().GetTargets()[0].name, j.GetBody1Rel().GetTargets()[0].name,
                    tuple(j.GetLocalPos0Attr().Get()), _wxyz(j.GetLocalRot0Attr().Get()),
                    tuple(j.GetLocalPos1Attr().Get()), _wxyz(j.GetLocalRot1Attr().Get()), j.GetAxisAttr().Get(),
                    j.GetLowerLimitAttr().Get(), j.GetUpperLimitAttr().Get())
        return [found[f"J{i}"] for i in range(1, 7)]

    def pedestal(self, path, height) -> None:
        m = mesh.Merged()
        m.box((0, 0, -height / 2), (1.0, 1.0, height), 0.03)  # under the arm's base, down to the floor
        m.box((0, 0, 0.015 - height), (1.3, 1.3, 0.03), 0.005)
        self.mesh(path, m.arrays(), self.paint("dark"))

    def _tool(self, path, chain) -> None:
        """The suction-cup boom on the arm's last link: a tube along the boom, a crossbar and four cups."""
        self.b.xform(path, ops=())
        xf = self.UsdGeom.Xformable(self.stage.GetPrimAtPath(path))
        xf.AddTransformOp().Set(_matrix(self.b.Gf, chain.tool))
        tube, cups = mesh.Merged(), mesh.Merged()
        boom, drop = plan.TOOL_BOOM, plan.TOOL_DROP
        tube.cylinder((-boom / 2, 0, drop - 0.03), 0.045, boom, "X", 12)
        tube.box((0.0, 0, (0.1 + drop) / 2), (0.05, 0.05, drop - 0.1), 0.006)
        tube.box((0.0, 0, 0.1), (0.08, 0.7, 0.05), 0.008)
        tube.box((-0.25, 0, 0.1), (0.5, 0.05, 0.05), 0.008)
        for x, y in ((-0.25, -0.3), (-0.25, 0.3), (0.25, -0.3), (0.25, 0.3)):
            cups.cylinder((x, y, 0.035), 0.05, 0.07, "Z", 12)
            tube.cylinder((x, y, 0.09), 0.012, 0.05, "Z", 6)
        self.mesh(f"{path}/Boom", tube.arrays(), self.paint("steel"))
        self.mesh(f"{path}/Cups", cups.arrays(), self.paint("rubber"))

    def conveyor(self, path, x_end, y, yaw=0.0) -> None:
        self.asset(path, "conveyor", (x_end, y, 0.0), yaw)

    def washer(self, path, at) -> None:
        """The blank washer at the head of the line: blanks come out of its slot."""
        body, dark, steel = mesh.Merged(), mesh.Merged(), mesh.Merged()
        body.box((0, 0, 1.0), (1.5, 1.8, 1.9), 0.04)
        dark.box((0.76, 0, plan.BELT_TOP + 0.06), (0.02, 1.0, 0.14), 0.0)
        for y in (-0.45, 0.45):
            steel.cylinder((0, y, 2.15), 0.12, 0.5, "Z", 12)
        steel.box((-0.76, 0, 1.4), (0.01, 1.2, 0.8), 0.0)
        self.b.xform(path, (at[0], at[1], 0.0))
        self.mesh(f"{path}/Body", body.arrays(), self.paint("cabinet"))
        self.mesh(f"{path}/Slot", dark.arrays(), self.paint("dark"))
        self.mesh(f"{path}/Pipes", steel.arrays(), self.paint("steel"))

    def blank_stack(self, path, at) -> None:
        self.b.xform(path, (at[0], at[1], 0.0))
        self.asset(f"{path}/Pallet", "pallet", (0, 0, 0), 90.0)
        m = mesh.Merged()
        m.box((0, 0, 0.144 + 0.28), (plan.BLANK_SIZE[0], plan.BLANK_SIZE[1], 0.56), 0.002)
        self.mesh(f"{path}/Stack", m.arrays(), self.paint("blank"))

    def stillage(self, path, at, scale=1.0) -> None:
        """An open steel stillage: corner posts and a frame the parts stack into."""
        frame = mesh.Merged()
        w, d, h = 1.15, 1.0, 0.95
        frame.box((0, 0, 0.14), (w, d, 0.05), 0.006)
        for sx in (-1, 1):
            for sy in (-1, 1):
                frame.box((sx * w / 2, sy * d / 2, h / 2), (0.05, 0.05, h), 0.006)
                frame.box((sx * w / 2, sy * d / 2, 0.06), (0.14, 0.14, 0.12), 0.006)
        for sy in (-1, 1):
            frame.box((0, sy * d / 2, h - 0.02), (w, 0.04, 0.04), 0.006)
        self.b.xform(path, (at[0], at[1], 0.0))
        self.mesh(f"{path}/Frame", frame.arrays(), self.paint("stillage"))

    def part_pool(self, root, kind, count) -> list:
        """Prims that hold parts, parked under the floor until used. All one mesh, written once per prim."""
        arrays = _blank_arrays() if kind == "blank" else _formed_arrays()
        out = []
        self.b.xform(root)
        for i in range(count):
            path = f"{root}/{kind}_{i}"
            self.b.xform(path, (0, 0, PARKED_Z), ops=("translate", "rotateZ"))
            self.mesh(f"{path}/Mesh", arrays, self.paint("blank" if kind == "blank" else "formed"))
            out.append(path)
        return out

    def lamps(self, root, at, machine) -> tuple:
        """A stack light with a rotating alarm beacon beside it, on a bracket. Each lamp is an unlit lens with a lit one
        over it that the visuals show or hide: on 6.2 a changed emissive intensity did not reach the render, while
        visibility does."""
        self.b.xform(root, at, ops=("translate",))
        bracket = mesh.Merged()
        bracket.box((0.03, -0.2, 0.0), (0.08, 0.6, 0.04), 0.005)  # to the column face behind it
        bracket.cylinder((0, 0, 0.2), 0.03, 0.4, "Z", 10)
        self.mesh(f"{root}/Bracket", bracket.arrays(), self.paint("dark"))
        lit = {}
        for i, color in enumerate(("green", "amber", "red")):
            rgb = self.layout.STATUS_COLORS[color]
            lit[color] = self._lens(f"{root}/{color}", (0, 0, 0.47 + i * 0.15), 0.075, 0.14, rgb, f"{machine}_{color}")
        return lit, self._beacon(root, machine)

    def _lens(self, path, at, radius, height, rgb, name) -> str:
        """An unlit lens and, over it, a lit one (hidden). Returns the lit one's path."""
        off, on = mesh.Merged(), mesh.Merged()
        off.cylinder(at, radius, height, "Z", 20, 0.01)
        on.cylinder(at, radius * 1.02, height * 1.02, "Z", 20, 0.01)
        unlit = self.b.material(f"Show_Lens_{name}", tuple(0.25 * c for c in rgb), roughness=0.3)
        glow = self.b.material(f"Show_Lit_{name}", rgb, roughness=0.3, emissive=rgb, intensity=LIT)
        self.mesh(f"{path}_off", off.arrays(), unlit)
        self.mesh(f"{path}_on", on.arrays(), glow)
        self.UsdGeom.Imageable(self.stage.GetPrimAtPath(f"{path}_on")).MakeInvisible()
        return f"{path}_on"

    def _beacon(self, root, machine) -> dict:
        Gf = self.b.Gf
        path = f"{root}/Beacon"
        self.b.xform(path, (0.0, -0.4, 0.02), ops=("translate",))
        rgb = self.layout.STATUS_COLORS["red"]
        on = self._lens(f"{path}/Dome", (0, 0, 0.17), 0.15, 0.3, rgb, f"{machine}_beacon")
        base = mesh.Merged()
        base.cylinder((0, 0, 0.0), 0.16, 0.04, "Z", 20)
        self.mesh(f"{path}/Base", base.arrays(), self.paint("dark"))
        spin = f"{path}/Spin"
        self.b.xform(spin, (0, 0, 0.17), ops=("rotateZ",))
        light = self.UsdLux.SphereLight.Define(self.stage, f"{spin}/Light")
        light.CreateRadiusAttr(0.12)
        light.CreateIntensityAttr(0.0)
        light.CreateColorAttr(Gf.Vec3f(*rgb))
        light.AddTranslateOp().Set(Gf.Vec3d(-0.3, 0.0, 0.0))  # out from the column, so it lights the press face
        return {"on": on, "light": f"{spin}/Light", "spin": spin}

    def screen(self, path, at, width, yaw, name) -> str:
        """A small lit screen in a dark housing, on a bracket. Returns the shader whose texture shows on it."""
        height = width * 9 / 16
        mat = self.b.material(f"Show_Screen_{name}", (0.02, 0.02, 0.02), roughness=0.3, emissive=(1, 1, 1),
                              intensity=350.0, texture="")
        self.b.xform(path, at, yaw)
        self.b.quad(f"{path}/Panel", width, height, mat)
        housing = mesh.Merged()
        housing.box((0, 0.04, 0), (width + 0.05, 0.07, height + 0.05), 0.008)
        self.mesh(f"{path}/Housing", housing.arrays(), self.paint("dark"))
        return f"{self.layout.LOOKS}/Show_Screen_{name}/Shader"


def _wxyz(q) -> tuple:
    im = q.GetImaginary()
    return (q.GetReal(), im[0], im[1], im[2])


def _matrix(Gf, m):
    """A numpy pose as USD's row-vector matrix."""
    return Gf.Matrix4d(*[float(v) for v in m.T.flatten()])


def _blank_arrays() -> dict:
    m = mesh.Merged()
    m.box((0, 0, 0.004), (plan.BLANK_SIZE[0], plan.BLANK_SIZE[1], 0.008), 0.002)
    return m.arrays()


def _formed_arrays() -> dict:
    points, normals, faces = mesh.stamped_panel(plan.BLANK_SIZE[0], plan.BLANK_SIZE[1], 0.11)
    points = points + np.array([0, 0, 0.11])  # its flange on top, the pocket reaching down to its own zero
    return {"points": points, "normals": normals, "counts": np.full(len(faces), 4), "indices": faces.flatten(),
            "uvs": points[:, :2]}


def dress(b, assets_root, data, log) -> Dressed:
    """The whole dressing on the stage `b` holds, as built by the factory's scene. Call before physics starts."""
    d = Dresser(b, assets_root, data, log)
    d.retire_old_visuals()
    b.xform(ROOT)
    d.hall()
    layout = d.layout
    out = Dressed(shots=plan.shots(layout))
    lines = plan.line_layouts(layout)
    for line in lines:
        out.lines.append(_dress_line(d, line, data))
    _dress_surroundings(d, out, lines)
    _dress_lights(d, lines)
    _check_no_new_physics(d)
    return out


def _dress_line(d, line, data) -> DressedLine:
    root = f"{ROOT}/{line.line.replace('-', '_')}"
    d.b.xform(root)
    geometry, paths = d.press_body(line)
    for i, x_end in enumerate(line.infeed_origins()):
        d.conveyor(f"{root}/Infeed{i}", x_end, line.infeed_y())
    d.conveyor(f"{root}/Outfeed", line.outfeed_origin(), line.outfeed_y())
    d.washer(f"{root}/Washer", line.washer())
    d.blank_stack(f"{root}/BlankStack", line.blank_stack())
    d.stillage(f"{root}/Stillage", line.stillage())
    loader = d.arm(f"{root}/Loader", line.loader_base(), plan.loader_motion, line)
    unloader = d.arm(f"{root}/Unloader", line.unloader_base(), plan.unloader_motion, line)
    x, y, z = geometry.beacon_at
    lamps, beacon = d.lamps(f"{paths['frame']}/Lamps", (x, y, z), line.machine.replace("-", "_"))
    tag = line.machine.replace("-", "_")
    ax, ay, az = geometry.andon_at
    hx, hy, hz = geometry.hmi_at
    screens = [d.screen(f"{paths['frame']}/Andon", (ax, ay, az), 0.95, 90.0 + 180.0, f"{tag}_andon"),
               d.screen(f"{paths['base']}/Hmi", (hx, hy, hz), 0.34, 0.0, f"{tag}_hmi")]
    spm = _strokes_per_min(data, line.machine)
    return DressedLine(line, geometry, paths["frame"], paths["slide"], paths["flywheel"], loader, unloader,
                       plan.belt_flow(line, spm), d.part_pool(f"{root}/Blanks", "blank", POOL_BLANKS),
                       d.part_pool(f"{root}/Formed", "formed", POOL_FORMED),
                       d.part_pool(f"{root}/Stacked", "formed", STILLAGE_PARTS), lamps, beacon, screens)


def _strokes_per_min(data, machine) -> float:
    kind = next(m["kind"] for m in data["machines"] if m["id"] == machine)
    return float(data["main_drives"][kind]["strokes_per_min"])


def _dress_surroundings(d, out, lines) -> None:
    """The forklift and its load; racks of stock along the walls."""
    path = f"{ROOT}/Forklift"
    d.b.xform(path)
    d.asset(f"{path}/Body", "forklift", (0, 0, 0), FORKLIFT_YAW)
    d.b.xform(f"{path}/Load", (1.75, 0, PARKED_Z), ops=("translate",))
    d.asset(f"{path}/Load/Pallet", "pallet", (0, 0, 0), 90.0)
    m = mesh.Merged()
    m.box((0, 0, 0.144 + 0.3), (plan.BLANK_SIZE[0], plan.BLANK_SIZE[1], 0.6), 0.002)
    d.mesh(f"{path}/Load/Stack", m.arrays(), d.paint("blank"))
    out.forklift, out.fork_load = path, f"{path}/Load"
    out.forklift_round = plan.forklift_round(lines)
    west = min(line.cx for line in lines) - RACK_ROW_X
    for i, y in enumerate(np.arange(-6.0, 22.0, 4.05)):
        d.asset(f"{ROOT}/Racks/Rack_{i}", "rack", (west, float(y), 0.0), 0.0, HALL_SCALE)


def _dress_lights(d, lines) -> None:
    Gf = d.b.Gf
    root = f"{ROOT}/Lights"
    d.b.xform(root)
    dome = d.UsdLux.DomeLight.Define(d.stage, f"{root}/Fill")
    dome.CreateIntensityAttr(DOME)
    dome.CreateColorAttr(Gf.Vec3f(0.9, 0.95, 1.0))
    for i, line in enumerate(lines):
        for j, x in enumerate((-13.0, -6.5, 0.0, 6.5)):
            rect = d.UsdLux.RectLight.Define(d.stage, f"{root}/Bay_{i}_{j}")
            rect.CreateWidthAttr(5.0)
            rect.CreateHeightAttr(2.0)
            rect.CreateIntensityAttr(BAY_LIGHT)
            rect.CreateColorTemperatureAttr(4800.0)
            rect.CreateEnableColorTemperatureAttr(True)
            rect.AddTranslateOp().Set(Gf.Vec3d(x, line.cy, 8.8))


def _check_no_new_physics(d) -> None:
    """Fails loudly if anything the dressing added would be simulated: the plant's physics must stay the factory's."""
    from pxr import Usd
    root = d.stage.GetPrimAtPath(ROOT)
    bad = [str(p.GetPath()) for p in Usd.PrimRange(root) if p.HasAPI(d.UsdPhysics.RigidBodyAPI)
           or p.HasAPI(d.UsdPhysics.ArticulationRootAPI) or p.IsA(d.UsdPhysics.Joint)]
    if bad:
        raise RuntimeError(f"the showcase added physics to the plant's stage: {bad[:5]}")
