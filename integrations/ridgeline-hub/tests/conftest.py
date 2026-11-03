import pytest

from ridgeline_hub_part.config import DEFAULT_HUB

NOT_PUBLIC = (
    f"the Ridgeline factory example ({DEFAULT_HUB}) is not here; it is not public yet, so these tests "
    "run only next to a private checkout of Sculptured-Examples"
)


def pytest_collection_modifyitems(config, items):
    """Every test reads the factory example: without it they are skipped, not failed."""
    if DEFAULT_HUB.is_dir():
        return
    for item in items:
        item.add_marker(pytest.mark.skip(reason=NOT_PUBLIC))


@pytest.fixture(autouse=True)
def saves_root(monkeypatch, tmp_path):
    root = tmp_path / "saves"
    monkeypatch.setenv("ARENA_SAVES", str(root))
    return root
