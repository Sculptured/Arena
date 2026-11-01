"""The part's config, and loading the factory hub's `market` module and supplier data."""
import hashlib
import importlib
import json
import sys
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from arena_part import errors

PACKAGE_DIR = Path(__file__).resolve().parent.parent
DEFAULT_HUB = (PACKAGE_DIR / ".." / ".." / ".." / "Sculptured-Examples" / "factory" / "hub").resolve()
DEFAULT_PLANT_START = "2026-03-10T06:00:00"


@dataclass(frozen=True)
class Config:
    hub_path: Path
    data_path: Path
    plant_start: datetime


def parse_config(config) -> Config:
    config = config or {}
    hub = Path(config.get("hub_path") or DEFAULT_HUB).resolve()
    data = Path(config.get("data_path") or hub / ".." / "data" / "ridgeline.json").resolve()
    raw = config.get("plant_start") or DEFAULT_PLANT_START
    try:
        start = datetime.fromisoformat(raw)
    except (TypeError, ValueError):
        raise errors.invalid(f"plant_start must be an ISO datetime, got {raw!r}") from None
    return Config(hub, data, start)


def load_market(cfg: Config):
    """Returns (the hub's `ridgeline_hub.market` module, its suppliers). Only `market` is imported:
    the hub's app pulls in FastAPI and httpx."""
    if not (cfg.hub_path / "ridgeline_hub" / "market.py").is_file():
        raise errors.invalid(f"hub_path {cfg.hub_path} has no ridgeline_hub/market.py")
    try:
        suppliers = json.loads(cfg.data_path.read_text())["suppliers"]
    except (OSError, ValueError, KeyError) as e:
        raise errors.invalid(f"data_path {cfg.data_path} has no readable suppliers: {e}") from None
    if str(cfg.hub_path) not in sys.path:
        sys.path.insert(0, str(cfg.hub_path))
    try:
        return importlib.import_module("ridgeline_hub.market"), suppliers
    except ImportError as e:
        raise errors.invalid(f"can't import ridgeline_hub.market from {cfg.hub_path}: {e}") from None


def source_files(cfg: Config) -> dict:
    """The files whose content decides the market's behaviour: name -> path."""
    return {"market.py": cfg.hub_path / "ridgeline_hub" / "market.py", "ridgeline.json": cfg.data_path}


def file_hash(path: Path) -> str:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return f"missing: {path}"


def source_hashes(cfg: Config) -> dict:
    return {name: file_hash(p) for name, p in source_files(cfg).items()}
