"""The part's config, and loading the factory hub's `market` module and supplier data."""
import json
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path

from arena_part import errors, params, sources

from . import inbound

PACKAGE_DIR = Path(__file__).resolve().parent.parent
DEFAULT_HUB = (PACKAGE_DIR / ".." / ".." / ".." / "Sculptured-Examples" / "factory" / "hub").resolve()
DEFAULT_PLANT_START = "2026-03-10T06:00:00"


@dataclass(frozen=True)
class Config:
    hub_path: Path
    data_path: Path
    plant_start: datetime
    inbound: tuple = ()  # raw-material trucks on their way (inbound.py)


def parse_config(config) -> Config:
    config = config or {}
    hub = Path(config.get("hub_path") or DEFAULT_HUB).resolve()
    data = Path(config.get("data_path") or hub / ".." / "data" / "ridgeline.json").resolve()
    deliveries = config.get("inbound", [])
    try:
        inbound.check_config(deliveries)
    except ValueError as e:
        raise errors.invalid(str(e)) from None
    return Config(hub, data, params.config_datetime(config, "plant_start", DEFAULT_PLANT_START), tuple(deliveries))


def load_market(cfg: Config):
    """Returns (the hub's `ridgeline_hub.market` module, its suppliers). Only `market` is imported:
    the hub's app pulls in FastAPI and httpx."""
    market = sources.import_read_only(cfg.hub_path, "ridgeline_hub.market", "hub_path")
    try:
        suppliers = json.loads(cfg.data_path.read_text())["suppliers"]
    except (OSError, ValueError, KeyError) as e:
        raise errors.invalid(f"data_path {cfg.data_path} has no readable suppliers: {e}") from None
    return market, suppliers


def source_files(cfg: Config) -> dict:
    """The files whose content decides the market's behaviour: name -> path."""
    return {"market.py": cfg.hub_path / "ridgeline_hub" / "market.py", "ridgeline.json": cfg.data_path}


def source_hashes(cfg: Config) -> dict:
    return sources.hashes(source_files(cfg))
