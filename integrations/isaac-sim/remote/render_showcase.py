"""Renders the plant on Isaac Sim without arenad, for looking at the showcase scene and for making its video.

    /isaac-sim/python.sh remote/render_showcase.py --seconds 95 --out showcase.mp4
    /isaac-sim/python.sh remote/render_showcase.py --seconds 3 --stills wide,press_a --out look.mp4

Run it with PYTHONPATH=<arena>/kits/python and ARENA_SAVES set (an MP4 path that is not absolute goes under
$ARENA_SAVES/isaac-sim/). It drives the same part arenad drives (start, advance, stop) with the arena seed, so the
plant runs its own scenario on its own clock. With --stills it renders no video: it steps physics to --seconds, then
writes one image of each named shot to --stills-dir."""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from isaac_part.backends import BACKENDS  # noqa: E402
from isaac_part.part import IsaacSim  # noqa: E402

CHUNK_MS = 500


def _parse_args() -> argparse.Namespace:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=float, default=95.0, help="arena seconds to run")
    ap.add_argument("--out", default="showcase.mp4")
    ap.add_argument("--camera", default="auto")
    ap.add_argument("--fps", type=int, default=30)
    ap.add_argument("--height", type=int, default=720)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--scene", default="showcase")
    ap.add_argument("--stills", default="", help="comma-separated shots: no video, one image of each")
    ap.add_argument("--stills-dir", default="stills")
    return ap.parse_args()


def main() -> None:
    args = _parse_args()
    part = IsaacSim(BACKENDS["isaac"])
    config = {"scene": args.scene, "record": args.out, "camera": args.camera, "record_fps": args.fps,
              "record_height": args.height}
    t0 = time.monotonic()
    part.start("render", config, args.seed, 0)
    print(f"[render] started in {time.monotonic() - t0:.1f} s", flush=True)
    plant = part.instances["render"]
    if args.stills:
        plant.video.every = 10 ** 9  # no frames on the way
    t1 = time.monotonic()
    end_ms = int(args.seconds * 1000)
    for ms in range(CHUNK_MS, end_ms + 1, CHUNK_MS):
        result = part.advance("render", ms)
        for event in result["events"]:
            print(f"[render] {ms / 1000:6.1f} s {event['kind']}: {event['text'][:90]}", flush=True)
        if ms % 10000 == 0:
            print(f"[render] arena {ms / 1000:.0f} s, wall {time.monotonic() - t1:.0f} s", flush=True)
    wall = time.monotonic() - t1
    if args.stills:
        plant.video.stills(plant, args.stills.split(","), Path(args.stills_dir))
    stats = {"arena_s": args.seconds, "wall_s": round(wall, 1), "wall_per_arena_s": round(wall / max(args.seconds, 1e-6), 2),
             **plant.video.stats()}
    part.stop("render")
    print("[render] " + json.dumps(stats), flush=True)


if __name__ == "__main__":
    main()
