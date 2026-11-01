"""Save folders by convention: `$ARENA_SAVES/<package name>/<save>/`."""
import os
import shutil
from pathlib import Path

from .serve import kept


def root(package: str) -> Path:
    return Path(os.environ.get("ARENA_SAVES", "saves")) / package


def drop(package: str, save: str) -> None:
    shutil.rmtree(root(package) / save, ignore_errors=True)


def gc(package: str, keep) -> dict:
    """Removes the package's save folders that `keep` does not keep: a `gc` result."""
    base = root(package)
    doomed = [p for p in base.iterdir() if p.is_dir() and not kept(keep, p.name)] if base.is_dir() else []
    freed = sum(f.stat().st_size for p in doomed for f in p.rglob("*") if f.is_file())
    for p in doomed:
        shutil.rmtree(p, ignore_errors=True)
    return {"dropped": len(doomed), "bytes": freed}
