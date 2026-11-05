"""Shared helpers: paths, YAML loading, ffmpeg and ffprobe calls, the look (colours, font)."""
import json
import os
import subprocess
import sys
from pathlib import Path

import yaml

# Work folder: ~/.cache/arena-showcase/video by default; SHOWCASE_VIDEO_WORK overrides it.
WORK = Path(os.environ.get("SHOWCASE_VIDEO_WORK", str(Path.home() / ".cache/arena-showcase/video")))
MODEL = Path(os.environ.get("SHOWCASE_VIDEO_MODEL", WORK / "models" / "en_GB-cori-high.onnx"))
W, H, FPS = 1920, 1080, 30
FONT = "Inter"
FONT_FILES = Path("/usr/share/fonts/opentype/inter")
BG = "#0d1117"
FG = "#e6edf3"
MUTED = "#8b949e"
ACCENT = "#58a6ff"
PALETTE = ["#58a6ff", "#f0883e", "#3fb950", "#d2a8ff", "#ff7b72", "#e3b341"]


def load_yaml(path):
    return yaml.safe_load(Path(path).read_text())


def load_json(path):
    return json.loads(Path(path).read_text())


def run(cmd, **kw):
    """Run a command at low priority; stop with its stderr tail when it fails."""
    cmd = ["nice", "-n", "10", *map(str, cmd)]
    p = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if p.returncode:
        sys.exit(f"command failed: {' '.join(cmd[:6])}...\n{p.stderr[-1500:]}")
    return p.stdout


def ffmpeg(*args):
    return run(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", *args])


def duration(path):
    out = run(["ffprobe", "-v", "error", "-show_entries", "format=duration",
               "-of", "csv=p=0", path])
    return float(out.strip())


def has_audio(path):
    out = run(["ffprobe", "-v", "error", "-select_streams", "a", "-show_entries",
               "stream=index", "-of", "csv=p=0", path])
    return bool(out.strip())


def ass_time(t):
    h, rem = divmod(max(t, 0), 3600)
    m, s = divmod(rem, 60)
    return f"{int(h)}:{int(m):02d}:{s:05.2f}"
