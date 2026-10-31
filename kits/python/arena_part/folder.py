"""Copying a part's folder into saves and back."""
import os
import shutil
from pathlib import Path


def copy_save(src: Path, dst: Path, prev: Path | None) -> int:
    """Copy `src` into `dst`. A file whose size and mtime match `prev`'s copy is hard-linked to it
    instead of copied. Returns the bytes newly written."""
    src, dst = Path(src), Path(dst)
    prev = Path(prev) if prev is not None else None
    written = 0
    dst.mkdir(parents=True, exist_ok=True)
    for root, dirs, files in os.walk(src):
        rel = Path(root).relative_to(src)
        (dst / rel).mkdir(parents=True, exist_ok=True)
        for name in files:
            s, d = Path(root) / name, dst / rel / name
            st = s.stat()
            p = prev / rel / name if prev is not None else None
            if p is not None and p.is_file() and not p.is_symlink():
                ps = p.stat()
                if ps.st_size == st.st_size and ps.st_mtime_ns == st.st_mtime_ns:
                    if d.exists():
                        d.unlink()
                    os.link(p, d)
                    continue
            if d.exists():
                d.unlink()
            shutil.copy2(s, d)  # keeps mtime, so the next save can link it
            written += st.st_size
    return written


def restore(save_dir: Path, dst: Path) -> None:
    """Copy a save into the live folder `dst`. Never links, so the live folder can't modify a save."""
    save_dir, dst = Path(save_dir), Path(dst)
    dst.mkdir(parents=True, exist_ok=True)
    for root, _dirs, files in os.walk(save_dir):
        rel = Path(root).relative_to(save_dir)
        (dst / rel).mkdir(parents=True, exist_ok=True)
        for name in files:
            d = dst / rel / name
            if d.exists() or d.is_symlink():
                d.unlink()
            shutil.copy2(Path(root) / name, d)
