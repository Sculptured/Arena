"""The part's config: where the factory's isaac folder and data are, the plant clock's start, the physics rate."""
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from arena_part import errors, params

PACKAGE_DIR = Path(__file__).resolve().parent.parent
DEFAULT_ISAAC = (PACKAGE_DIR / ".." / ".." / ".." / "Sculptured-Examples" / "factory" / "isaac").resolve()
DEFAULT_PLANT_START = "2026-03-10T06:00:00"


@dataclass(frozen=True)
class Config:
    isaac_path: Path
    data_path: Path
    plant_start: datetime
    physics_hz: int | None  # None: the factory's own press_model.PHYSICS_HZ
    scenario: bool


def parse_config(config) -> Config:
    config = config or {}
    isaac = Path(config.get("isaac_path") or DEFAULT_ISAAC).resolve()
    data = Path(config.get("data_path") or isaac / ".." / "data" / "ridgeline.json").resolve()
    hz = config.get("physics_hz")
    if hz is not None and (not isinstance(hz, int) or isinstance(hz, bool) or hz <= 0):
        raise errors.invalid(f"physics_hz must be a positive integer, got {hz!r}")
    scenario = config.get("scenario", True)
    if not isinstance(scenario, bool):
        raise errors.invalid(f"scenario must be true or false, got {scenario!r}")
    return Config(isaac, data, params.config_datetime(config, "plant_start", DEFAULT_PLANT_START), hz, scenario)
