"""The factory's code, read only: the files its behaviour depends on (the fingerprint) and importing them."""
from pathlib import Path
from types import SimpleNamespace

from arena_part import errors, sources

from .config import Config, parse_config

# The plain-Python modules Factory's behaviour depends on, in every backend.
COMMON = ("factory.py", "press_model.py", "signals.py", "paths.py", "schedule.py", "layout.py")
DATA = "ridgeline.json"
# This package's own folder.
ADAPTER_DIR = Path(__file__).resolve().parent


def files(cfg: Config, backend) -> dict:
    """name -> path of every source file behind the plant: the common modules, the data, the backend's own."""
    out = {name: cfg.isaac_path / name for name in COMMON + backend.files}
    out[DATA] = cfg.data_path
    return out


def declared(backend) -> dict:
    """The hashes `describe` declares: the files at the default paths."""
    return sources.hashes(files(parse_config({}), backend))


def check(cfg: Config, backend) -> None:
    """Every file must be there (invalid otherwise) and hash as declared (-32003 otherwise)."""
    sources.check_unchanged(declared(backend), files(cfg, backend), "check isaac_path and data_path")


def adapter_hashes() -> dict:
    """The adapter's own source files (`isaac_part/*.py`) by name: how it drives the factory is behaviour
    too."""
    return sources.hashes({p.name: p for p in sorted(ADAPTER_DIR.glob("*.py"))})


def load(cfg: Config) -> SimpleNamespace:
    """The factory's `factory` and `press_model` modules, imported from isaac_path."""
    def imp(module):
        return sources.import_read_only(cfg.isaac_path, module, "isaac_path")
    return SimpleNamespace(factory=imp("factory"), press_model=imp("press_model"))


def default_physics_hz():
    """press_model.PHYSICS_HZ at the default path, or None when it can't be imported (describe still answers)."""
    try:
        return load(parse_config({})).press_model.PHYSICS_HZ
    except errors.PartError:
        return None
