"""The part's config: where the factory's isaac folder and data are, the plant clock's start, the physics rate, and
the recording."""
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from arena_part import errors, params

from . import equipment

PACKAGE_DIR = Path(__file__).resolve().parent.parent
SCENES = ("ridgeline", "showcase")
DEFAULT_ISAAC = (PACKAGE_DIR / ".." / ".." / ".." / "Sculptured-Examples" / "factory" / "isaac").resolve()
DEFAULT_PLANT_START = "2026-03-10T06:00:00"


@dataclass(frozen=True)
class Config:
    isaac_path: Path
    data_path: Path
    plant_start: datetime
    physics_hz: int | None  # None: the factory's own press_model.PHYSICS_HZ
    scenario: bool
    record: str | None = None  # the MP4 to render to (isaac only); relative to the part's save folder
    camera: str = "auto"  # the factory's camera: a rig in layout.CAMERAS, "auto" (cuts to events) or "free"
    record_fps: int = 30  # frames per second of arena time; the MP4 plays at this rate, so in arena time
    scene: str = "ridgeline"  # what the plant looks like: the factory's own scene, or "showcase" (visuals only)
    record_height: int = 720  # the MP4's height in pixels; the width follows at 16:9
    cells: tuple = ()  # robot cells the arena adds beside the factory's presses (equipment.py)
    power: dict | None = None  # the plant's load model; None: no load is reported


def parse_config(config) -> Config:
    config = config or {}
    isaac = Path(config.get("isaac_path") or DEFAULT_ISAAC).resolve()
    data = Path(config.get("data_path") or isaac / ".." / "data" / "ridgeline.json").resolve()
    hz = config.get("physics_hz")
    scenario = config.get("scenario", True)
    if not isinstance(scenario, bool):
        raise errors.invalid(f"scenario must be true or false, got {scenario!r}")
    return Config(isaac, data, params.config_datetime(config, "plant_start", DEFAULT_PLANT_START),
                  _positive(hz, "physics_hz"), scenario, **_recording(config), **_equipment(config))


def _equipment(config) -> dict:
    """`cells` (default none) and `power` (default none), checked for shape."""
    cells, power = config.get("cells", []), config.get("power")
    try:
        equipment.check_config(cells, power)
    except ValueError as e:
        raise errors.invalid(str(e)) from None
    return {"cells": tuple(cells), "power": power}


def _positive(n, name):
    if n is not None and (not isinstance(n, int) or isinstance(n, bool) or n <= 0):
        raise errors.invalid(f"{name} must be a positive integer, got {n!r}")
    return n


def _recording(config) -> dict:
    """`record` (a non-empty path), `camera` (a name), `record_fps`, `scene` and `record_height`. Whether the camera
    exists is checked once the factory's layout is loaded (recording.check_camera)."""
    record, camera = config.get("record"), config.get("camera", "auto")
    if record is not None and (not isinstance(record, str) or not record.strip()):
        raise errors.invalid(f"record must be a path to an MP4, got {record!r}")
    if not isinstance(camera, str):
        raise errors.invalid(f"camera must be a name, got {camera!r}")
    scene = config.get("scene", "ridgeline")
    if scene not in SCENES:
        raise errors.invalid(f"scene must be one of {', '.join(SCENES)}, got {scene!r}")
    return {"record": record, "camera": camera, "record_fps": _positive(config.get("record_fps", 30), "record_fps"),
            "scene": scene, "record_height": _positive(config.get("record_height", 720), "record_height")}
