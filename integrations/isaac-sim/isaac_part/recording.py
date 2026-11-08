"""Recording the plant to MP4 (config `record`): where the file goes, which camera, and when frames fall.

A frame is rendered every `physics_hz / record_fps` physics steps, so the MP4 runs in arena time whatever the
render costs. Rendering is the backend's (only `isaac` can); this module is plain Python."""
from pathlib import Path

from arena_part import errors

CAMERA_MODES = ("auto", "free")
STILL_SETTLE_FRAMES = 32  # frames a still waits after a cut, for the renderer's history to catch up


def path_for(record: str, root: Path, instance: str) -> Path:
    """The MP4 of one instance: `record` (relative to `root`, the part's save folder), with `-<instance>` before
    its suffix, since arenad runs a base and then a branch on the same config."""
    p = Path(record)
    p = p if p.is_absolute() else root / p
    return p.with_name(f"{p.stem}-{instance}{p.suffix or '.mp4'}")


def check_camera(camera: str, cameras) -> None:
    if camera not in CAMERA_MODES and camera not in cameras:
        names = ", ".join(sorted(cameras) + list(CAMERA_MODES))
        raise errors.invalid(f"camera must be one of {names}, got {camera!r}")


def frame_every(physics_hz: int, fps: int) -> int:
    """Physics steps between frames (at least one)."""
    return max(1, round(physics_hz / fps))


def frame_size(height: int) -> tuple:
    """Width and height at 16:9, both even, as H.264 wants them."""
    even = height + height % 2
    width = even * 16 // 9
    return width + width % 2, even


def next_frame(steps: int, every: int) -> int:
    """The step count of the next frame after `steps`."""
    return (steps // every + 1) * every
