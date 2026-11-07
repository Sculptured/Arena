"""The core the integration tests use.

- ``ARENA_SDK_URL``: an arenad that is already running (any benchmark); or
- an arenad the tests start (``Arena.local``) with the retail example benchmark, from
  ``ARENA_SDK_BIN_DIR`` (default ``core/target/release``, then ``core/target/debug``; it needs
  ``arenad``, ``arena-retail`` and ``arena-postgres``) on the Postgres at ``ARENA_SDK_PG_URL``
  (default ``deploy/compose.yml``'s, ``postgres://arena:arena@127.0.0.1:5544/postgres``).

The arenad the tests start cleans up unreferenced saves when it starts, like any arenad, so point
``ARENA_SDK_PG_URL`` at a Postgres server no other test run is using at the same time.

Without either, the tests marked ``core`` are skipped. They drive the API directly (start and finish
external runs, call a tool endpoint, fork, read records, export) and never pose as an agent.
"""

from __future__ import annotations

import os
import socket
import tempfile
from pathlib import Path
from urllib.parse import urlparse

import pytest

from arena import Arena

REPO = Path(__file__).resolve().parents[3]
NEEDED = ("arenad", "arena-retail", "arena-postgres")


def _bin_dir() -> str | None:
    cands = [os.environ.get("ARENA_SDK_BIN_DIR"), REPO / "core/target/release", REPO / "core/target/debug"]
    for c in cands:
        if c and all((Path(c) / n).is_file() for n in NEEDED):
            return str(c)
    return None


def _reachable(pg_url: str) -> bool:
    u = urlparse(pg_url)
    try:
        with socket.create_connection((u.hostname or "127.0.0.1", u.port or 5432), timeout=2):
            return True
    except OSError:
        return False


@pytest.fixture(scope="session")
def arena():
    url = os.environ.get("ARENA_SDK_URL")
    if url:
        a = Arena(url)
        try:
            a.health()
        except Exception as e:  # noqa: BLE001
            pytest.skip(f"ARENA_SDK_URL={url} does not answer: {e}")
        yield a
        a.close()
        return
    bin_dir = _bin_dir()
    pg = os.environ.get("ARENA_SDK_PG_URL", "postgres://arena:arena@127.0.0.1:5544/postgres")
    if bin_dir is None:
        pytest.skip("no arenad build (set ARENA_SDK_BIN_DIR or ARENA_SDK_URL)")
    if not _reachable(pg):
        pytest.skip(f"no Postgres at {urlparse(pg).hostname}:{urlparse(pg).port} (set ARENA_SDK_PG_URL)")
    saves = tempfile.mkdtemp(prefix="arena-sdk-test-saves-")
    a = Arena.local(
        benchmark="retail",
        packages=[str(REPO / "integrations"), str(REPO / "benchmarks")],
        bin_dir=bin_dir,
        pg_url=pg,
        saves=saves,
        cors_origin="http://127.0.0.1:5173",
    )
    yield a
    a.close()


@pytest.fixture
def cleanup(arena):
    """Runs to delete after the test, even when it fails."""
    runs: list[str] = []
    yield runs
    for r in runs:
        try:
            arena.delete_run(r)
        except Exception:  # noqa: BLE001
            pass
