"""The GPU and driver the isaac fingerprint records. Read from the driver's /proc files, which don't query the
GPU; nvidia-smi only when those are absent. arenad compares fingerprints exactly, so a transient failure is an
error, never a recorded null: null means there is no NVIDIA driver at all."""
import re
import shutil
import subprocess
from pathlib import Path

from arena_part import errors

PROC = Path("/proc/driver/nvidia")
NO_GPU = {"gpu": None, "driver": None}
SMI = ["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"]
SMI_TIMEOUT_S = 10


def from_proc(root: Path) -> dict | None:
    """{gpu, driver} from `<root>/version` and `<root>/gpus/*/information` (several GPUs: their models joined
    with ", ", in bus order), or None when either is missing."""
    try:
        version = (root / "version").read_text()
        infos = [p.read_text() for p in sorted(root.glob("gpus/*/information"))]
    except OSError:
        return None
    driver = re.search(r"Kernel Module(?: for \S+)?\s+(\d+(?:\.\d+)+)", version)
    models = [m.group(1).strip() for i in infos if (m := re.search(r"^Model:\s*(.+)$", i, re.M))]
    if driver is None or not models:
        return None
    return {"gpu": ", ".join(models), "driver": driver.group(1)}


def from_smi(cmd=SMI, timeout=SMI_TIMEOUT_S) -> dict:
    """{gpu, driver} from nvidia-smi; -32603 when it fails or doesn't answer in time."""
    try:
        out = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        raise errors.internal(f"nvidia-smi did not answer within {timeout} s, so the GPU fingerprint can't be "
                              "recorded now; try again when the GPU is less busy") from None
    except OSError as e:
        raise errors.internal(f"nvidia-smi failed: {e}") from None
    rows = out.stdout.strip().splitlines()
    if out.returncode != 0 or not rows:
        raise errors.internal(f"nvidia-smi failed (exit {out.returncode}): {out.stderr.strip()[:200]}")
    models = [r.split(",")[0].strip() for r in rows]
    return {"gpu": ", ".join(models), "driver": rows[0].split(",")[-1].strip()}


def gpu_host(root: Path = PROC, smi=from_smi) -> dict:
    found = from_proc(root)
    if found is not None:
        return found
    if not root.exists() and shutil.which("nvidia-smi") is None:
        return dict(NO_GPU)
    return smi()
