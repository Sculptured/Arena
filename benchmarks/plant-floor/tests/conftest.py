import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / "integrations" / "plant-opcua"), str(ROOT / "integrations" / "factory-01-erp")]


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    monkeypatch.setenv("ARENA_SAVES", str(tmp_path / "saves"))
