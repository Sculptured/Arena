"""Renders the plant on Isaac Sim without arenad, for looking at the showcase scene and for making its video.

    /isaac-sim/python.sh remote/render_showcase.py --seconds 95 --out showcase.mp4
    /isaac-sim/python.sh remote/render_showcase.py --seconds 3 --stills wide,press_a --out look.mp4
    /isaac-sim/python.sh remote/render_showcase.py --shift --calm --seconds 40 --at "10:safe_stop_cell:{\\"cell\\":\\"CELL-R1\\"}"

Run it with PYTHONPATH=<arena>/kits/python and ARENA_SAVES set (an MP4 path that is not absolute goes under
$ARENA_SAVES/isaac-sim/). It drives the same part arenad drives (start, advance, call_tool, stop) with the arena seed,
so the plant runs its own scenario on its own clock. With --stills it renders no video: it steps physics to --seconds,
then writes one image of each named shot to --stills-dir.

--shift adds the factory benchmark's shift config (robot cell R1, the plant's load: benchmarks/factory/data/shift.json),
which brings the shift's scene pieces. --at "<seconds>:<tool>:<json args>" (repeatable) calls a plant tool at that
arena time, as a benchmark's setup or link would (show_status for the boards, safe_stop_cell): the world's own inputs,
for looking at the pieces in each state. --path-traced renders with the RTX path tracer (--spp samples per pixel)."""
import argparse
import json
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from isaac_part.backends import BACKENDS  # noqa: E402
from isaac_part.part import IsaacSim  # noqa: E402

CHUNK_MS = 500
SHIFT = Path(__file__).resolve().parents[3] / "benchmarks" / "factory" / "data" / "shift.json"


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
    ap.add_argument("--shift", action="store_true", help="the factory benchmark's shift config")
    ap.add_argument("--calm", action="store_true", help="no press wears (plant_scenario calm)")
    ap.add_argument("--at", action="append", default=[], help="<seconds>:<tool>:<json args>")
    ap.add_argument("--path-traced", action="store_true")
    ap.add_argument("--spp", type=int, default=64)
    ap.add_argument("--setting", action="append", default=[], help="/carb/path=json value, for the renderer")
    return ap.parse_args()


def _inputs(args) -> list:
    """(arena ms, tool, args) of the plant tool calls asked for, in time order."""
    out = [(0, "plant_scenario", {"name": "calm"})] if args.calm else []
    for spec in args.at:
        seconds, tool, body = spec.split(":", 2)
        out.append((int(float(seconds) * 1000), tool, json.loads(body)))
    return sorted(out, key=lambda c: c[0])


def _path_traced(spp: int) -> None:
    import carb
    s = carb.settings.get_settings()
    s.set("/rtx/rendermode", "PathTracing")
    s.set("/rtx/pathtracing/spp", spp)
    s.set("/rtx/pathtracing/totalSpp", spp)
    s.set("/rtx/pathtracing/optixDenoiser/enabled", True)


def _run(part, args, end_ms) -> None:
    inputs = _inputs(args)
    t1 = time.monotonic()
    for ms in range(CHUNK_MS, end_ms + 1, CHUNK_MS):
        while inputs and inputs[0][0] <= ms:
            at, tool, body = inputs.pop(0)
            out = part.call_tool("render", tool, body, at)
            print(f"[render] {at / 1000:6.1f} s {tool} -> {json.dumps(out.get('error') or out.get('result'))[:90]}",
                  flush=True)
        for event in part.advance("render", ms)["events"]:
            print(f"[render] {ms / 1000:6.1f} s {event['kind']}: {event['text'][:90]}", flush=True)
        if ms % 10000 == 0:
            print(f"[render] arena {ms / 1000:.0f} s, wall {time.monotonic() - t1:.0f} s", flush=True)


def main() -> None:
    args = _parse_args()
    part = IsaacSim(BACKENDS["isaac"])
    config = {"scene": args.scene, "record": args.out, "camera": args.camera, "record_fps": args.fps,
              "record_height": args.height}
    if args.shift:
        config.update(json.loads(SHIFT.read_text())["plant"])
    if args.setting:
        config["render"] = {k: json.loads(v) for k, v in (x.split("=", 1) for x in args.setting)}
    t0 = time.monotonic()
    part.start("render", config, args.seed, 0)
    print(f"[render] started in {time.monotonic() - t0:.1f} s", flush=True)
    if args.path_traced:
        _path_traced(args.spp)
    plant = part.instances["render"]
    if args.stills:
        plant.video.every = 10 ** 9  # no frames on the way
    t1 = time.monotonic()
    _run(part, args, int(args.seconds * 1000))
    wall = time.monotonic() - t1
    if args.stills:
        plant.video.stills(plant, args.stills.split(","), Path(args.stills_dir))
    stats = {"arena_s": args.seconds, "wall_s": round(wall, 1), "wall_per_arena_s": round(wall / max(args.seconds, 1e-6), 2),
             **plant.video.stats()}
    part.stop("render")
    print("[render] " + json.dumps(stats), flush=True)


if __name__ == "__main__":
    main()
