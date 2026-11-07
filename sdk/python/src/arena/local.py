"""Start an arenad of your own as a subprocess (``Arena.local``)."""

from __future__ import annotations

import os
import shutil
import signal
import socket
import subprocess
import tempfile
import time
from typing import Any, Mapping, Optional, Sequence

from .client import Arena
from .errors import ArenaError, ConnectionFailed


class StartFailed(ArenaError):
    """arenad exited or didn't answer while starting; the message ends with its log."""


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def find_arenad(arenad: Optional[str], bin_dir: Optional[str]) -> str:
    """``arenad`` as given, else ``<bin_dir>/arenad``, else ``$ARENA_BIN_DIR/arenad``, else on PATH."""
    for cand in (arenad, bin_dir and os.path.join(bin_dir, "arenad"),
                 os.environ.get("ARENA_BIN_DIR") and os.path.join(os.environ["ARENA_BIN_DIR"], "arenad"),
                 shutil.which("arenad")):
        if cand and os.path.isfile(cand) and os.access(cand, os.X_OK):
            return cand
    raise StartFailed(
        "no arenad binary: pass arenad=<path> or bin_dir=<folder>, or set ARENA_BIN_DIR "
        "(build it with `cd core && cargo build --release -p arenad`)"
    )


class LocalArena(Arena):
    """An :class:`Arena` connected to an arenad this object started. Use it in a ``with`` block (or
    call :meth:`close`) so the process stops.

    Args:
        benchmark: the benchmark package new runs use (``ARENA_BENCHMARK``), e.g. ``"retail"``.
        packages: folders whose subfolders hold ``arena-package.toml`` (``ARENA_PACKAGES``); default:
            arenad's own (``integrations`` and ``benchmarks`` of its checkout).
        pg_url: the Postgres server for arenad's records (``ARENA_PG_URL``); default arenad's
            (``postgres://arena:arena@127.0.0.1:5544/postgres``, ``deploy/compose.yml``).
        saves: where saves go (``ARENA_SAVES``); default a temporary folder removed on close.
        bin_dir: what ``${ARENA_BIN_DIR}`` means in package manifests, and where to find arenad.
        arenad: the arenad binary (default: found in ``bin_dir``, ``$ARENA_BIN_DIR`` or PATH).
        port: the port (default: a free one).
        max_db_bytes: the saves' disk limit (``ARENA_MAX_DB_BYTES``).
        cors_origin: web origins allowed to call the API from a browser (``ARENA_CORS_ORIGIN``).
        env: more environment variables for arenad (model provider keys, ``ARENA_LOCAL_MODELS``, ...);
            arenad otherwise inherits this process's environment.
        log: a file for arenad's output (default: a temporary file; ``self.log_path``).
        startup_timeout: seconds to wait for arenad to answer.
    """

    def __init__(
        self,
        benchmark: Optional[str] = None,
        packages: Optional[Sequence[str]] = None,
        pg_url: Optional[str] = None,
        saves: Optional[str] = None,
        bin_dir: Optional[str] = None,
        arenad: Optional[str] = None,
        port: Optional[int] = None,
        max_db_bytes: Optional[int] = None,
        cors_origin: Optional[str] = None,
        env: Optional[Mapping[str, str]] = None,
        log: Optional[str] = None,
        startup_timeout: float = 120.0,
        **client: Any,
    ):
        binary = find_arenad(arenad, bin_dir)
        self.port = port or free_port()
        self._tmp_saves = None
        if saves is None:
            self._tmp_saves = tempfile.mkdtemp(prefix="arena-saves-")
            saves = self._tmp_saves
        e = dict(os.environ)
        e.update(
            {
                "ARENA_PORT": str(self.port),
                "ARENA_SAVES": os.path.abspath(saves),
            }
        )
        if benchmark:
            e["ARENA_BENCHMARK"] = benchmark
        if packages:
            e["ARENA_PACKAGES"] = os.pathsep.join(os.path.abspath(p) for p in packages)
        if pg_url:
            e["ARENA_PG_URL"] = pg_url
        if bin_dir:
            e["ARENA_BIN_DIR"] = os.path.abspath(bin_dir)
        if max_db_bytes is not None:
            e["ARENA_MAX_DB_BYTES"] = str(max_db_bytes)
        if cors_origin:
            e["ARENA_CORS_ORIGIN"] = cors_origin
        if env:
            e.update(env)
        if log is None:
            fd, log = tempfile.mkstemp(prefix="arenad-", suffix=".log")
            os.close(fd)
        self.log_path = log
        self._log = open(log, "ab")
        self.process = subprocess.Popen(
            [binary], env=e, stdout=self._log, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL
        )
        super().__init__(f"http://127.0.0.1:{self.port}", **client)
        try:
            self._wait_ready(startup_timeout)
        except BaseException:
            self.close()
            raise

    def _wait_ready(self, timeout: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise StartFailed(f"arenad exited with {self.process.returncode}:\n{self.log_tail()}")
            try:
                self._http.get("/api/health", timeout=5.0).raise_for_status()
                return
            except Exception:
                time.sleep(0.3)
        raise StartFailed(f"arenad did not answer within {timeout} s:\n{self.log_tail()}")

    def log_tail(self, lines: int = 30) -> str:
        try:
            with open(self.log_path, encoding="utf-8", errors="replace") as f:
                return "".join(f.readlines()[-lines:])
        except OSError:
            return ""

    def close(self) -> None:
        """Stops arenad (ctrl-c, then kill after 15 s) and removes a temporary saves folder."""
        super().close()
        p = getattr(self, "process", None)
        if p is not None and p.poll() is None:
            p.send_signal(signal.SIGINT)
            try:
                p.wait(15)
            except subprocess.TimeoutExpired:
                p.kill()
                p.wait(5)
        if getattr(self, "_log", None):
            self._log.close()
        if self._tmp_saves:
            shutil.rmtree(self._tmp_saves, ignore_errors=True)
            self._tmp_saves = None

    def __repr__(self) -> str:
        return f"LocalArena({self.url!r}, pid={self.process.pid})"


__all__ = ["LocalArena", "StartFailed", "ConnectionFailed", "free_port", "find_arenad"]
