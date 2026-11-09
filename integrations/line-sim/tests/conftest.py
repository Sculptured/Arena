import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "line2-oee" / "data" / "plant.json"


def config(**study) -> dict:
    """The config the line2-oee benchmark gives this part, with study settings overridden."""
    c = json.loads(DATA.read_text())
    c["study"] = {**c["study"], **study}
    return c


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
