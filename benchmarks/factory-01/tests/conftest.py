import sys
from pathlib import Path

import pytest

ERP = Path(__file__).resolve().parents[3] / "integrations" / "factory-01-erp"
sys.path.insert(0, str(ERP))


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    monkeypatch.setenv("ARENA_SAVES", str(tmp_path / "saves"))
