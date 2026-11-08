import pytest

CONFIG = {"meter": "Plant 2 main meter", "site": "Plant 2 (Stamping)", "plant_start": "2026-03-10T06:00:00",
          "initial_kw": 1500, "tariff": {"standard_per_kwh": 0.1, "peak_per_kwh": 0.4}}


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
