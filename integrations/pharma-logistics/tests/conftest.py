import json
from pathlib import Path

import pytest

DATA = Path(__file__).resolve().parents[3] / "benchmarks" / "pharma" / "data" / "larchmont.json"
QA_ACTIONS = ["batch_release", "batch_hold", "deviation"]


def logistics_config() -> dict:
    """The config the pharma benchmark gives this part, from its data file."""
    d = json.loads(DATA.read_text())
    customers = [{"name": c["name"], "region": c["region"]} for c in d["customers"]]
    return {"logistics": d["logistics"], "customers": customers, "today": d["today"], "defer": QA_ACTIONS}


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
