import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "factory-01" / "data" / "factory-01.json"


def config() -> dict:
    """The config the factory-01 benchmark gives this part."""
    return json.loads(DATA.read_text())


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
