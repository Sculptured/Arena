"""Puts the plant's state on the showcase's stage, once per frame: every movement is a transform written here from the
plant's own readings (crank angle, ram position, vibration, status), never a PhysX body. Only reads the Factory."""
import importlib
import math

from pxr import Gf, Sdf, UsdGeom

from . import showcase_camera as camera
from . import showcase_hmi as hmi
from . import showcase_layout as plan
from . import showcase_motion as motion
from .showcase_scene import PARKED_Z, STILLAGE_PARTS, _matrix

FLYWHEEL_SHOWN = 0.2  # of the real 5 turns a second: at the true speed, 30 frames a second would only strobe it
BEACON_LIGHT = 40000.0
BEACON_HZ = 1.5
BEACON_DEG_PER_S = 360.0
SCREEN_REFRESH_S = 1.0
STACK_STEP = 0.05
LAMP_FOR = {"running": "green", "degraded": "amber", "tripped": "red", "locked-out": "red", "maintenance": "amber"}
ALARM = ("tripped", "locked-out")


class _Writer:
    """Attribute writes that skip unchanged values, with each attribute looked up once."""

    def __init__(self, stage, log):
        self.stage, self.log = stage, log
        self.attrs, self.values, self.missing = {}, {}, set()

    def set(self, path, name, value) -> None:
        key = (path, name)
        if self.values.get(key) == value:
            return
        attr = self.attrs.get(key)
        if attr is None:
            prim = self.stage.GetPrimAtPath(path)
            attr = prim.GetAttribute(name) if prim else None
            if attr is None or not attr.IsValid():
                if key not in self.missing:
                    self.missing.add(key)
                    self.log(f"[showcase] missing {path}.{name}")
                return
            self.attrs[key] = attr
        self.values[key] = value
        attr.Set(value)


class ShowcaseVisuals:
    def __init__(self, stage, dressed, camera_mode, label_dir, log):
        self.stage, self.dressed, self.log = stage, dressed, log
        self.write = _Writer(stage, log)
        self.layout = importlib.import_module("layout")
        idle = camera.SHIFT_ESTABLISHING if dressed.shift is not None else camera.ESTABLISHING
        self.director = camera.Director(dressed.shots, camera_mode, idle)
        self.shift = None
        if dressed.shift is not None:
            from .showcase_shift_visuals import ShiftVisuals
            self.shift = ShiftVisuals(dressed.shift, self.write, self._show, label_dir)
        self.screens = hmi.Screens(label_dir)
        self.last_now, self.next_screen = None, {}
        self.visible = {}
        for line in dressed.lines:
            line.flow.warm(4)

    def show(self, factory, events, now, extras=None) -> None:
        dt = 1 / 30 if self.last_now is None else max(0.0, now - self.last_now)
        self.last_now = now
        vs = factory.visual_state()
        shake_a = 0.0
        for line in self.dressed.lines:
            amplitude = self._line(line, factory, vs, now, dt)
            if line.plan.machine == "PR-A1":
                shake_a = amplitude
        self._forklift(now)
        self._logistics(vs)
        if self.shift is not None:
            events = list(events) + self.shift.update(extras, now)
        self._camera(events, now, shake_a)

    # ---- the press lines ---------------------------------------------------------------------------------------

    def _line(self, line, factory, vs, now, dt) -> float:
        machine = line.plan.machine
        pr = factory.presses[machine]
        m = vs["machines"][machine]
        cycle = motion.crank_cycle(pr.shaft_angle, pr.p["crank_ratio"])
        line.flow.update(cycle, dt)
        amplitude = motion.shake_amplitude(m["sensors"]["vibration_mm_s"], now)
        self._press(line, pr, amplitude, now)
        self._arm(line.loader, cycle)
        self._arm(line.unloader, cycle)
        self._parts(line)
        self._lamps(line, m["status"], now)
        self._screens(line, vs, m, now)
        return amplitude

    def _press(self, line, pr, amplitude, now) -> None:
        dx, dy, dz = motion.shake_offset(amplitude, now)
        write, g = self.write, line.geometry
        write.set(line.frame, "xformOp:translate", Gf.Vec3d(dx, dy, dz))
        height = max(0.0, pr.ram_pos) * g.stroke / pr.p["stroke_m"]
        write.set(line.slide, "xformOp:translate", Gf.Vec3d(dx, dy, dz + height))
        c = g.flywheel_center
        write.set(line.flywheel, "xformOp:translate", Gf.Vec3d(c[0] + dx, c[1] + dy, c[2] + dz))
        write.set(line.flywheel, "xformOp:rotateY", math.degrees(pr.shaft_angle * FLYWHEEL_SHOWN) % 360.0)

    def _arm(self, arm, cycle) -> None:
        for link, pose in arm.chain.links(arm.cycle.at(cycle)).items():
            path = arm.links.get(link)
            if path:
                self.write.set(path, "xformOp:transform", _matrix(Gf, pose))

    def _tip(self, arm, cycle):
        """Where a held part is, in the world, and which way its long side lies (degrees)."""
        tool = arm.chain.tool_pose(arm.cycle.at(cycle))
        return ((tool[0, 3] + arm.base[0], tool[1, 3] + arm.base[1], tool[2, 3] + arm.base[2]),
                math.degrees(math.atan2(tool[1, 0], tool[0, 0])))

    def _parts(self, line) -> None:
        flow, p = line.flow, line.plan
        ix0, _ = p.infeed_x()
        ox0, _ = p.outfeed_x()
        die = p.die()
        blanks = [((ix0 + d, p.infeed_y(), plan.BELT_TOP + 0.03), 0.0) for d in flow.infeed.parts]
        formed = [((ox0 + d, p.outfeed_y(), plan.BELT_TOP + 0.03), 90.0) for d in flow.outfeed.parts]
        if flow.loader_has:
            blanks.append(self._tip(line.loader, flow.cycle))
        if flow.unloader_has:
            formed.append(self._tip(line.unloader, flow.cycle))
        if flow.die == "blank":
            blanks.append((die, 0.0))
        elif flow.die == "formed":
            formed.append((die, 0.0))
        self._place(line.blanks, blanks)
        self._place(line.formed, formed)
        self._stack(line)

    def _place(self, pool, placed) -> None:
        for i, path in enumerate(pool):
            at, yaw = placed[i] if i < len(placed) else ((0.0, 0.0, PARKED_Z), 0.0)
            self.write.set(path, "xformOp:translate", Gf.Vec3d(*at))
            self.write.set(path, "xformOp:rotateZ", float(yaw))

    def _stack(self, line) -> None:
        """The stillage's stack, one part higher for each part that arrived (a full one starts again: swapped)."""
        n = (line.flow.stillage % STILLAGE_PARTS) + 1
        x, y = line.plan.stillage()
        for i, path in enumerate(line.stacked):
            z = 0.17 + i * STACK_STEP if i < n else PARKED_Z
            self.write.set(path, "xformOp:translate", Gf.Vec3d(x, y, z))

    def _lamps(self, line, status, now) -> None:
        lit = LAMP_FOR.get(status)
        for color, path in line.lamps.items():
            self._show(path, color == lit)
        alarm = status in ALARM
        flash = alarm and (now * BEACON_HZ) % 1.0 < 0.6  # a rotating beacon reads as flashing from one place
        beacon = line.beacon
        self._show(beacon["on"], flash)
        self.write.set(beacon["light"], "inputs:intensity", BEACON_LIGHT if flash else 0.0)
        if alarm:
            self.write.set(beacon["spin"], "xformOp:rotateZ", (now * BEACON_DEG_PER_S) % 360.0)

    def _screens(self, line, vs, m, now) -> None:
        if now < self.next_screen.get(line.plan.line, 0.0):
            return
        self.next_screen[line.plan.line] = now + SCREEN_REFRESH_S
        png = self.screens.file_for(_reading(line.plan, vs, m, now))
        for shader in line.screens:
            self.write.set(shader, "inputs:diffuse_texture", Sdf.AssetPath(png))
            self.write.set(shader, "inputs:emissive_color_texture", Sdf.AssetPath(png))

    # ---- what moves around them ---------------------------------------------------------------------------------

    def _forklift(self, now) -> None:
        d = self.dressed
        x, y, yaw, fork, carrying = d.forklift_round.pose(now)
        self.write.set(d.forklift, "xformOp:translate", Gf.Vec3d(x, y, 0.0))
        self.write.set(d.forklift, "xformOp:rotateZ", float(yaw))
        self.write.set(d.fork_load, "xformOp:translate", Gf.Vec3d(1.75, 0.0, fork if carrying else PARKED_Z))

    def _show(self, path, on) -> None:
        if self.visible.get(path) != on:
            prim = self.stage.GetPrimAtPath(path)
            if prim:
                self.visible[path] = on
                img = UsdGeom.Imageable(prim)
                img.MakeVisible() if on else img.MakeInvisible()

    def _logistics(self, vs) -> None:
        """The spindle crate and the truck, where the plant says they are."""
        crate, truck = vs["crate"], vs["truck"]
        path = self.layout.PRIMS["crate"]["mover"]
        self._show(path, crate["visible"])
        if crate["visible"]:
            x, y = crate["xy"]
            self.write.set(path, "xformOp:translate", Gf.Vec3d(x, y, crate["z"]))
            self.write.set(path, "xformOp:rotateZ", math.degrees(crate["yaw"] or 0.0))
        tpath = self.layout.PRIMS["truck"]["mover"]
        self._show(tpath, truck["visible"])
        self.write.set(tpath, "xformOp:translate", Gf.Vec3d(self.layout.DOCK["truck_x"], truck["y"], 0.0))

    # ---- the camera ---------------------------------------------------------------------------------------------

    def _camera(self, events, now, shake) -> None:
        shot, fraction = self.director.update(events, now)
        self._aim_at(shot, fraction, now, shake)

    def _aim_at(self, shot, fraction, now, shake) -> None:
        eye = camera.eye_at(shot, fraction)
        target = shot.target
        if shot.follow == "forklift":
            x, y, *_ = self.dressed.forklift_round.pose(now)
            target = (x, y, 1.2)
        elif shot.follow == "amr" and self.shift is not None:
            target = self.shift.amr_pose(now)
        dx, dy, dz = motion.shake_offset(shake * shot.shake, now + 0.37)
        eye = (eye[0] + dx, eye[1] + dy, eye[2] + dz)
        main = self.layout.PRIMS["main_camera"]
        matrix = Gf.Matrix4d().SetLookAt(Gf.Vec3d(*eye), Gf.Vec3d(*target), Gf.Vec3d(0, 0, 1)).GetInverse()
        self.write.set(main, "xformOp:transform", matrix)
        self.write.set(main, "focalLength", float(shot.focal))

    def aim(self, name) -> None:
        """A named shot now, for a still."""
        self._aim_at(self.dressed.shots[name], 0.5, self.last_now or 0.0, 0.0)


def _reading(p, vs, m, now) -> hmi.Reading:
    """What the line's screens show: the plant's own numbers. Vibration is blank while the drives spin up."""
    line_doc = vs["lines"][p.line]
    order = vs["orders"].get(line_doc.get("current_order")) if line_doc.get("current_order") else None
    s = m["sensors"]
    return hmi.Reading(line_doc["name"], p.machine, m["status"], s["strokes_per_min"],
                       None if now < motion.WARM_S else s["vibration_mm_s"], s["spindle_temp_c"],
                       order["done"] if order else m["strokes"], order["qty"] if order else 0,
                       order["item"] if order else "")
