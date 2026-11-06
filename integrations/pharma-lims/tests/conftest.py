import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "pharma" / "data" / "larchmont.json"
QA_ACTIONS = ["batch_release", "batch_hold", "deviation"]


def lims_config() -> dict:
    """The config the pharma benchmark gives this part, from its data file."""
    d = json.loads(DATA.read_text())
    return {"lims": d["lims"], "lots": d["lots"], "today": d["today"], "decides": QA_ACTIONS}


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
