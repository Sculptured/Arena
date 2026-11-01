"""Code an integration loads from a folder it doesn't own: importing it without writing there, and the
source hashes a fingerprint declares."""
import hashlib
import importlib
import sys
from pathlib import Path

from . import errors


def import_read_only(folder, module: str, label: str):
    """Imports `module` (dotted) from `folder`, which goes first on `sys.path`. The folder is treated as
    read only: no `__pycache__` is written for any module imported after this. A missing file or a failed
    import is `invalid`, naming the folder as config key `label`. Modules are process-global, so one
    process serves one folder."""
    folder = Path(folder)
    rel = Path(*module.split(".")).with_suffix(".py")
    if not (folder / rel).is_file():
        raise errors.invalid(f"{label} {folder} has no {rel.as_posix()}")
    sys.dont_write_bytecode = True
    if str(folder) not in sys.path:
        sys.path.insert(0, str(folder))
    try:
        return importlib.import_module(module)
    except ImportError as e:
        raise errors.invalid(f"can't import {module} from {folder}: {e}") from None


def file_hash(path) -> str:
    """sha256 of a file's bytes, or `missing: <path>`."""
    try:
        return hashlib.sha256(Path(path).read_bytes()).hexdigest()
    except OSError:
        return f"missing: {path}"


def hashes(files: dict) -> dict:
    """name -> path becomes name -> file_hash."""
    return {name: file_hash(p) for name, p in files.items()}


def check_unchanged(declared: dict, files: dict, hint: str = "") -> None:
    """The files (name -> path) must be there (`invalid` naming the first that isn't, with `hint` on where
    to look) and hash as `declared` (name -> hash), or replays would differ: -32003 naming the first file
    that doesn't. Call it before importing any of them, so a refused start loads nothing."""
    for name, path in files.items():
        if not Path(path).is_file():
            raise errors.invalid(f"{name} not found at {path}" + (f" ({hint})" if hint else ""))
    for name, actual in hashes(files).items():
        if actual != declared.get(name):
            raise errors.fingerprint(f"{name} at {files[name]} differs from the one the fingerprint declares")
