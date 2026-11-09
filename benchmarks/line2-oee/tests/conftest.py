import sys
from pathlib import Path

import pytest

SIM = Path(__file__).resolve().parents[3] / "integrations" / "line-sim"
sys.path.insert(0, str(SIM))


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    monkeypatch.setenv("ARENA_SAVES", str(tmp_path / "saves"))
