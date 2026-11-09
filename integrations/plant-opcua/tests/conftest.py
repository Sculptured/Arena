import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "plant-floor" / "data" / "plant-floor.json"


def config() -> dict:
    """The plant config the plant-floor benchmark gives this part."""
    return json.loads(DATA.read_text())["plant"]


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    monkeypatch.setenv("ARENA_SAVES", str(tmp_path / "saves"))
