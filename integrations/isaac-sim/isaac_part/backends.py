"""What the Factory's PlantIO is: the factory's offline double (`standin`) or Isaac Sim (`isaac`), and how each
steps, saves and restores. The backend is a process argument because `describe` has no config and the backend
changes the fingerprint."""
import functools
import subprocess
from dataclasses import dataclass
from typing import Callable

from arena_part import sources

from . import isaac_backend


@dataclass(frozen=True)
class Backend:
    name: str
    files: tuple  # factory files its behaviour depends on, beyond source.COMMON
    make: Callable  # (cfg, data, physics_hz) -> PlantIO
    run: Callable  # (io, steps, dt, physics_step): `steps` physics steps, each followed by physics_step(dt)
    get_state: Callable  # io -> what an approximate save keeps of the simulation
    set_state: Callable  # (io, state, factory) -> None
    host: Callable  # () -> what the fingerprint records of the machine
    max_live: int | None = None  # live instances one process can hold (None: any number)


def _standin_make(cfg, data, _physics_hz):
    fake_sim = sources.import_read_only(cfg.isaac_path / "tests", "fake_sim", "isaac_path")
    return fake_sim.FakeIO(data)


def _standin_run(io, steps, dt, physics_step):
    """The order of test_scenario.py's Rig.run and of the app (physics, then the post-step callback)."""
    for _ in range(steps):
        io.step(dt)
        physics_step(dt)


def _standin_get(io) -> dict:
    return {"presses": io.presses, "robot": io.robot, "people": io.people}


def _standin_set(io, state, _factory) -> None:
    io.presses, io.robot, io.people = state["presses"], state["robot"], state["people"]


@functools.cache
def gpu_host() -> dict:
    """{gpu, driver} from nvidia-smi, null when there is none. Asked once per process."""
    try:
        out = subprocess.run(["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"],
                             capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        return {"gpu": None, "driver": None}
    first = out.stdout.strip().splitlines()[0] if out.returncode == 0 and out.stdout.strip() else ""
    name, _, driver = first.partition(",")
    return {"gpu": name.strip() or None, "driver": driver.strip() or None}


STANDIN = Backend("standin", ("tests/fake_sim.py", "tests/press_dynamics.py"), _standin_make, _standin_run,
                  _standin_get, _standin_set, dict)
ISAAC = Backend("isaac", ("sim_io.py", "ridgeline_scene.py"), isaac_backend.make, isaac_backend.run,
                isaac_backend.get_state, isaac_backend.set_state, gpu_host, max_live=1)
BACKENDS = {b.name: b for b in (STANDIN, ISAAC)}
