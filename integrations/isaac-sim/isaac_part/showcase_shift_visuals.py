"""Puts the shift's state on its scene pieces, once per frame, from the plant's equipment (robot cell R1, the load) and
its signage (the utility's event, the OT network's state, a late truck): the welding arm's cycle and its parking on a
safe stop, the spark, the stack lights, the AMRs' rounds and the boards. It also says when something worth a cut
happened, as `shift` cue events for the camera director. Reads only; writes transforms and textures."""
import math

from pxr import Gf, Sdf

from . import showcase_robot as robot
from . import showcase_shift_layout as plan
from . import showcase_shift_screens as screens
from .showcase_scene import _matrix

SPARK_LIGHT = 9000.0
FLASH_HZ = 1.5
SCREEN_REFRESH_S = 1.0
LOAD_CUE_KW = 100.0  # a load change at least this big cuts to the energy board


def _flash(now) -> bool:
    return (now * FLASH_HZ) % 1.0 < 0.6


def _cue(name: str) -> dict:
    return {"kind": "shift", "text": name, "data": {"cue": name}}


class ShiftVisuals:
    def __init__(self, shift, write, show, label_dir):
        self.shift, self.write, self.show_prim = shift, write, show
        self.boards = screens.Boards(label_dir)
        self.prev = {}  # what the last frame saw: cell status, signage values, load
        self.stopped_at, self.started_at = None, None
        self.held_q = None  # the arm's angles when it last changed state
        self.current_q = None
        self.next_screen = 0.0

    def update(self, extras, now) -> list:
        """Everything for this frame; returns the cue events of what changed since the last one."""
        extras = extras or {}
        cell = (extras.get("cells") or {}).get("CELL-R1") or {"status": "running"}
        signage = extras.get("signage") or {}
        running = cell.get("status") == "running"
        q = self._arm(running, now)
        self._spark(running, q, now)
        self._lamps(running, signage, now)
        self._amrs(now)
        self._screens(extras, cell, signage, now)
        return self._cues(cell, signage, extras.get("power") or {})

    # ---- robot cell R1 -----------------------------------------------------------------------------------------

    def _arm(self, running, now):
        arm = self.shift.arm
        cycle_q = arm.cycle.motion.at(now / plan.WELD_CYCLE_S)
        if running != self.prev.get("arm_running", True):
            self.held_q = cycle_q if self.current_q is None else self.current_q
            self.stopped_at = None if running else now
            self.started_at = now if running else None
        if running:
            start = self.started_at
            f = 1.0 if start is None else robot.smooth((now - start) / plan.PARK_S)
            q = (self.held_q if self.held_q is not None else cycle_q) * (1 - f) + cycle_q * f
        else:
            f = robot.smooth((now - self.stopped_at) / plan.PARK_S)
            q = self.held_q * (1 - f) + arm.cycle.park * f
        self.prev["arm_running"] = running
        self.current_q = q
        for link, pose in arm.chain.links(q).items():
            path = arm.links.get(link)
            if path:
                self.write.set(path, "xformOp:transform", _matrix(Gf, pose))
        return q

    def _spark(self, running, q, now) -> None:
        arm = self.shift.arm
        welding = running and plan.burning(arm.cycle, now / plan.WELD_CYCLE_S)
        flicker = 0.55 + 0.45 * abs(math.sin(now * 53.0)) * abs(math.sin(now * 31.0 + 1.3))
        self.show_prim(arm.glow, welding)
        self.write.set(arm.spark, "inputs:intensity", SPARK_LIGHT * flicker if welding else 0.0)

    def _lamps(self, running, signage, now) -> None:
        ot = (signage.get("ot") or {}).get("value")
        exposed = running and ot == "intrusion"
        lit = "red" if exposed else ("green" if running else "amber")
        for color, path in self.shift.cell_lamps.items():
            self.show_prim(path, color == lit and (not exposed or _flash(now)))
        beacon = self.shift.cell_beacon
        self.show_prim(beacon["on"], exposed and _flash(now))
        self.write.set(beacon["light"], "inputs:intensity", 40000.0 if exposed and _flash(now) else 0.0)
        gw = {"intrusion": "red", "isolated": "amber", "revoked": "amber"}.get(ot, "green")
        for color, path in self.shift.gateway_lamps.items():
            self.show_prim(path, color == gw and (gw != "red" or _flash(now)))

    # ---- around the plant ----------------------------------------------------------------------------------------

    def _amrs(self, now) -> None:
        for path, route in self.shift.amrs:
            x, y, yaw, *_ = route.pose(now)
            self.write.set(path, "xformOp:translate", Gf.Vec3d(x, y, 0.0))
            self.write.set(path, "xformOp:rotateZ", float(yaw))

    def amr_pose(self, now) -> tuple:
        x, y, *_ = self.shift.amrs[0][1].pose(now)
        return (x, y, 0.5)

    def _screens(self, extras, cell, signage, now) -> None:
        if now < self.next_screen:
            return
        self.next_screen = now + SCREEN_REFRESH_S
        s = self.shift
        for shader, board in ((s.energy_screen, screens.energy(extras.get("power"), signage)),
                              (s.gateway_screen, screens.gateway(signage)), (s.dock_screen, screens.dock(signage)),
                              (s.cell_screen, screens.cell(cell, signage))):
            png = Sdf.AssetPath(self.boards.file_for(board))
            self.write.set(shader, "inputs:diffuse_texture", png)
            self.write.set(shader, "inputs:emissive_color_texture", png)

    # ---- cues ----------------------------------------------------------------------------------------------------

    def _cues(self, cell, signage, power) -> list:
        out = []
        status = cell.get("status")
        if "cell" in self.prev and status != self.prev["cell"]:
            out.append(_cue("cell_start" if status == "running" else "cell_stop"))
        self.prev["cell"] = status
        for key in ("dr", "ot", "truck"):
            value = (signage.get(key) or {}).get("value")
            if value is not None and value != self.prev.get(key) and _cue_name(key, value):
                out.append(_cue(_cue_name(key, value)))
            self.prev[key] = value
        kw = power.get("kw")
        last = self.prev.get("kw")
        if kw is not None and last is not None and abs(kw - last) >= LOAD_CUE_KW:
            out.append(_cue("load"))
        if kw is not None and (last is None or abs(kw - last) >= LOAD_CUE_KW):
            self.prev["kw"] = kw
        return out


def _cue_name(key: str, value: str):
    """The cue a board's new value calls for: the event's notice or window, the network's state, a late truck."""
    if key == "dr":
        return value if value in ("notified", "active") else None
    return "truck" if key == "truck" else value
