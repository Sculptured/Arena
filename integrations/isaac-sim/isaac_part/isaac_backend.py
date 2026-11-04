"""The `isaac` backend: sim_io.IsaacIO on a headless Isaac Sim 6.2, set up as the factory's run.py does.

UNVERIFIED: this has not run on Isaac Sim (no Isaac image or 16 GB GPU where it was written). It follows run.py
and sim_io.py; the calls run.py doesn't make are marked `unverified on 6.2`. Isaac Sim has one stage per process,
so one live plant per process (Backend.max_live)."""
import importlib
import importlib.util
import math

from arena_part import errors, sources

from .log import log

MISSING = ("Isaac Sim is not importable here: run this integration with Isaac Sim's Python on the DGX Spark "
           "(see README)")
_APP = {}  # the process's SimulationApp, the newest physics callback's generation, and where it forwards each step


def _idle(_dt):
    """The callback's target while no plant is being run: steps never reach a stopped plant."""


def _app():
    """The one SimulationApp of the process (headless; omni imports come after it)."""
    if "app" not in _APP:
        from isaacsim import SimulationApp
        # The renderer and size are run.py's recording settings; without a recording nothing is rendered.
        _APP["app"] = SimulationApp({"headless": True, "sync_loads": True, "renderer": "RaytracedLighting",
                                     "width": 1280, "height": 720})
        import carb
        carb.settings.get_settings().set("/rtx/post/dlss/execMode", 2)
    return _APP["app"]


def _stop_timeline():
    import omni.timeline
    omni.timeline.get_timeline_interface().stop()  # unverified on 6.2


def _stage(app, cfg, data):
    """A new stage with the factory's scene, as run.py builds it. The previous plant's timeline is stopped first."""
    import omni.usd
    from isaacsim.storage.native import get_assets_root_path
    scene = sources.import_read_only(cfg.isaac_path, "ridgeline_scene", "isaac_path")
    ctx = omni.usd.get_context()
    _stop_timeline()
    ctx.new_stage()
    app.update()
    root = get_assets_root_path()
    scene.build_scene(ctx.get_stage(), root, use_assets=root is not None, data=data)
    for _ in range(20):
        app.update()
    return ctx.get_stage()


def load(_cfg):
    """Isaac Sim must be importable. A save holds plain values and numpy arrays only, so nothing is imported yet:
    sim_io needs the SimulationApp running first."""
    if importlib.util.find_spec("isaacsim") is None:
        raise errors.internal(MISSING)


def make(cfg, data, physics_hz):
    load(cfg)
    _APP["step"] = _idle
    app = _app()
    import isaacsim.core.experimental.utils.app as app_utils
    sim_io = sources.import_read_only(cfg.isaac_path, "sim_io", "isaac_path")
    layout = importlib.import_module("layout")
    stage = _stage(app, cfg, data)
    people = layout.TECHNICIANS + list(layout.OPERATOR_IDS.values())
    io = sim_io.IsaacIO(stage, list(layout.MACHINES), layout.ROBOTS, people, data, "glide", log=log)
    sim_io.setup_physics(physics_hz, "cuda:0")
    app_utils.play()
    app_utils.update_app(steps=2)
    io.start()
    _register_callback(sim_io)
    return io


def _register_callback(sim_io) -> None:
    """A physics callback for this plant's stage. On 6.2 (RTX 3090, 2026-11-04) the callback registered for an
    earlier stage no longer ran after ctx.new_stage(), so each plant registers its own; one that does survive
    forwards nothing, since only the newest generation reaches the plant."""
    gen = _APP["gen"] = _APP.get("gen", 0) + 1
    sim_io.on_physics_step(lambda dt: _APP["step"](dt) if _APP["gen"] == gen else None)


def stop(_io):
    _APP["step"] = _idle
    if "app" in _APP:
        _stop_timeline()


def run(io, steps, dt, physics_step):
    """PhysX steps without rendering; Factory.physics_step runs in the post-step callback, as in run.py."""
    _APP["step"] = physics_step
    importlib.import_module("sim_io").step_physics(steps)


def _robot(io, r) -> dict:
    if r in io.kinematic:
        return {"kinematic": dict(io.kinematic[r])}
    view = io.robot_views[r]
    pos, rot = view.get_world_poses()
    return {"pos": pos.numpy(), "rot": rot.numpy(), "q": view.get_dof_positions().numpy(),
            "v": view.get_dof_velocities().numpy()}


def _person(io, person) -> tuple:
    from pxr import UsdGeom
    path = importlib.import_module("layout").PRIMS[person]["mover"]
    ops = {op.GetOpName(): op for op in UsdGeom.Xformable(io.stage.GetPrimAtPath(path)).GetOrderedXformOps()}
    t = ops["xformOp:translate"].Get()
    return float(t[0]), float(t[1]), float(ops["xformOp:rotateZ"].Get() or 0.0)


def get_state(io) -> dict:
    """Joint positions and velocities, IsaacIO's step count (kinematic robots integrate from it), the shaft unwrap,
    the last IMU readings, robots and people. Approximate: PhysX's solver warm start and contact caches are not
    saved."""
    return {"q": io.presses.get_dof_positions().numpy(), "v": io.presses.get_dof_velocities().numpy(),
            "steps": io.steps, "unwrapped": io.unwrapped.copy(),
            "last_raw": None if io.last_raw is None else io.last_raw.copy(), "last_accel": dict(io.last_accel),
            "robots": {r: _robot(io, r) for r in io.robots}, "people": {p: _person(io, p) for p in io.people}}


def _set_robot(io, r, s) -> None:
    if "kinematic" in s:
        io.kinematic[r] = dict(s["kinematic"])
        return
    view = io.robot_views[r]
    view.set_world_poses(s["pos"], s["rot"])  # unverified on 6.2
    view.set_dof_positions(s["q"])  # unverified on 6.2
    view.set_dof_velocities(s["v"])  # unverified on 6.2


def set_state(io, s, factory) -> None:
    io.presses.set_dof_positions(s["q"])  # unverified on 6.2
    io.presses.set_dof_velocities(s["v"])  # unverified on 6.2
    io.steps, io.unwrapped, io.last_raw, io.last_accel = s["steps"], s["unwrapped"], s["last_raw"], s["last_accel"]
    io.max_efforts = io.vel_targets = None  # the next command writes every drive again
    for r, robot in s["robots"].items():
        _set_robot(io, r, robot)
    for p, (x, y, yaw_deg) in s["people"].items():
        io.move_person(p, x, y, math.radians(yaw_deg), False)
    for pr in factory.presses.values():
        if pr.sent_unbalance is not None:
            io.set_unbalance(pr.id, pr.sent_unbalance)
