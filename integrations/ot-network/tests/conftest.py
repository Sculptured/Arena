import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "factory" / "data" / "shift.json"


def config() -> dict:
    """The config the factory benchmark gives this part, from its data file."""
    return json.loads(DATA.read_text())["ot_network"]


def front(name: str) -> dict:
    return json.loads(DATA.read_text())["fronts"][name]


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
