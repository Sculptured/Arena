#!/usr/bin/env python3
"""Editor: assemble a 1920x1080 narrated video from an edit file with ffmpeg.

    uv run build.py edit.yaml [--keep]

edit.yaml (see README.md): output, narration (timings.json from narrate.py), watermark, music,
and scenes of type title, image or clip. Each scene is rendered to its own segment (video,
narration, burned-in captions, lower-third, speed note), the segments are joined, and chapter
markers go into the metadata. Music is mixed in only when `music:` names a file.
"""
import argparse
import shutil
import tempfile
from pathlib import Path

import cards
import overlay
from common import BG, FPS, H, W, WORK, duration, ffmpeg, load_json, load_yaml

LEAD, TAIL = 0.8, 1.2
FADE = 0.25
OUT_ARGS = ["-c:v", "libx264", "-preset", "medium", "-crf", "18", "-pix_fmt", "yuv420p",
            "-r", FPS, "-c:a", "aac", "-b:a", "192k", "-ar", "48000", "-ac", "2"]


def scene_length(sc, narr):
    """Seconds the scene lasts: narration plus lead-in and tail, or its own `duration`."""
    if narr:
        return LEAD + narr["duration"] + TAIL
    return float(sc.get("duration", 3.0))


def motion_filter(kind, frames):
    z = {"zoom_in": f"1+0.10*on/{frames}", "zoom_out": f"1.10-0.10*on/{frames}",
         "pan_left": "1.10", "pan_right": "1.10", "none": "1"}[kind]
    x = {"pan_left": f"(iw-iw/zoom)*(1-on/{frames})",
         "pan_right": f"(iw-iw/zoom)*on/{frames}"}.get(kind, "iw/2-(iw/zoom/2)")
    return (f"zoompan=z='{z}':x='{x}':y='ih/2-(ih/zoom/2)':d={frames}:s={W}x{H}:fps={FPS}")


def image_chain(sc, total, tmp):
    src = tmp / f"{sc['id']}.png"
    if sc["type"] == "title" and sc.get("code"):
        cards.code_card(sc.get("title", ""), sc["code"], src)
    elif sc["type"] == "title":
        cards.title_card(sc.get("title", ""), sc.get("subtitle", ""), src)
    else:
        cards.image_canvas(sc["src"], src, sc.get("margin", 60), 180 if sc.get("safe_ui") else 0)
    frames = int(total * FPS) + 1
    inp = ["-framerate", FPS, "-loop", "1", "-t", f"{total:.3f}", "-i", src]
    return inp, motion_filter(sc.get("motion", "zoom_in"), frames), 1.0


def clip_chain(sc, total):
    src = sc["src"]
    start = float(sc.get("start", 0))
    clip_d = duration(src) - start
    speed = 1.0
    if clip_d > total:
        speed = min(clip_d / total, sc.get("max_speed", 8))
        if clip_d / speed > total + 0.05:
            print(f"  warning: {sc['id']} trimmed, needs more than {speed:g}x")
    fit_h = H - 180 if sc.get("safe_ui") else H
    y = "0" if sc.get("safe_ui") else "(oh-ih)/2"
    fit = (f"scale={W}:{fit_h}:force_original_aspect_ratio=decrease,"
           f"pad={W}:{H}:(ow-iw)/2:{y}:color={BG},setsar=1,fps={FPS},"
           f"setpts=PTS/{speed:.4f}")
    if clip_d / speed < total:
        fit += f",tpad=stop_mode=clone:stop_duration={total - clip_d / speed:.3f}"
    return ["-ss", str(start), "-i", src], fit, speed


def video_chain(sc, total, tmp):
    return clip_chain(sc, total) if sc["type"] == "clip" else image_chain(sc, total, tmp)


def render_scene(sc, narr, edit, tmp, out):
    total = scene_length(sc, narr)
    inp, vf, speed = video_chain(sc, total, tmp)
    ass = tmp / f"{sc['id']}.ass"
    ass.write_text(overlay.scene_ass(
        total, narr["captions"] if narr else (), LEAD, sc.get("lower_third"), speed,
        edit.get("watermark")))
    fonts = "/usr/share/fonts/opentype/inter"
    vf += (f",format=yuv420p,fade=t=in:d={FADE},fade=t=out:st={total - FADE:.3f}:d={FADE},"
           f"ass={ass}:fontsdir={fonts}")
    audio = audio_args(narr, total)
    ffmpeg(*inp, *audio[0], "-filter_complex",
           f"[0:v]{vf}[v];{audio[1]}", "-map", "[v]", "-map", "[a]", "-t", f"{total:.3f}",
           *OUT_ARGS, out)


def audio_args(narr, total):
    if not narr:
        return [], f"anullsrc=r=48000:cl=stereo,atrim=0:{total:.3f}[a]"
    delay = int(LEAD * 1000)
    return (["-i", narr["wav"]],
            f"[1:a]aresample=48000,aformat=channel_layouts=stereo,adelay={delay}|{delay},"
            f"apad=whole_dur={total:.3f}[a]")


def concat(segments, out, tmp):
    lst = tmp / "list.txt"
    lst.write_text("".join(f"file '{s}'\n" for s in segments))
    ffmpeg("-f", "concat", "-safe", "0", "-i", lst, "-c", "copy", out)


def chapters_meta(scenes, segments, tmp):
    lines, t = [";FFMETADATA1"], 0.0
    for sc, seg in zip(scenes, segments):
        d = duration(seg)
        if sc.get("chapter"):
            lines += ["[CHAPTER]", "TIMEBASE=1/1000", f"START={int(t * 1000)}",
                      f"END={int((t + d) * 1000)}", f"title={sc['chapter']}"]
        t += d
    meta = tmp / "chapters.txt"
    meta.write_text("\n".join(lines) + "\n")
    return meta


def finish(joined, meta, edit, out):
    music = edit.get("music")
    args = ["-i", joined, "-i", meta]
    maps = ["-map_metadata", "1", "-map", "0:v", "-map", "0:a"]
    if music:
        args += ["-stream_loop", "-1", "-i", music]
        vol = edit.get("music_volume", 0.07)
        d = duration(joined)
        fc = (f"[2:a]volume={vol},afade=t=in:d=2,afade=t=out:st={d - 3:.2f}:d=3[m];"
              f"[0:a][m]amix=inputs=2:duration=first:normalize=0[a]")
        maps = ["-map_metadata", "1", "-map", "0:v", "-map", "[a]", "-filter_complex", fc]
    else:
        maps += ["-af", "loudnorm=I=-16:TP=-1.5:LRA=7"]
    ffmpeg(*args, *maps, "-c:v", "copy", "-c:a", "aac", "-b:a", "192k", "-shortest",
           "-movflags", "+faststart", "-metadata", f"title={edit.get('title', '')}", out)


def narration_by_id(edit):
    if not edit.get("narration"):
        return {}
    return {s["id"]: s for s in load_json(edit["narration"])["scenes"]}


def build(edit_path, keep=False):
    edit = load_yaml(edit_path)
    base = Path(edit_path).resolve().parent
    for key in ("narration", "music"):
        if edit.get(key) and not Path(edit[key]).is_absolute():
            edit[key] = str(base / edit[key])
    out = Path(edit["output"])
    out.parent.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)
    narr = narration_by_id(edit)
    tmp = Path(tempfile.mkdtemp(dir=WORK))
    try:
        segs = []
        for sc in edit["scenes"]:
            seg = tmp / f"seg_{sc['id']}.mp4"
            print(f"scene {sc['id']} ({sc['type']})")
            render_scene(sc, narr.get(sc["id"]), edit, tmp, seg)
            segs.append(seg)
        joined = tmp / "joined.mp4"
        concat(segs, joined, tmp)
        finish(joined, chapters_meta(edit["scenes"], segs, tmp), edit, out)
        print(f"wrote {out} ({duration(out):.1f}s)")
    finally:
        if not keep:
            shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("edit")
    ap.add_argument("--keep", action="store_true", help="keep the temporary segments")
    a = ap.parse_args()
    build(a.edit, a.keep)


if __name__ == "__main__":
    main()
