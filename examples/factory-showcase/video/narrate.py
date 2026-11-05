#!/usr/bin/env python3
"""Narration: turn a script of scenes into one WAV per scene (Piper, offline) and timings.json.

    uv run narrate.py script.yaml [--out DIR]

script.yaml:  voice: <optional model path>;  scenes: [{id, text}, ...]
Writes DIR/<id>.wav and DIR/timings.json (default DIR: WORK/narration/<script name>).
Caption timing is estimated: Piper gives no word alignment, so each scene's duration is split
over its phrases in proportion to their length.
"""
import argparse
import json
import re
import sys
import wave
from pathlib import Path

from common import MODEL, WORK, load_yaml, run

MAX_CAPTION_CHARS = 84


def synth(text, model, wav):
    run([sys.executable, "-m", "piper", "-m", model, "-f", wav, "--sentence-silence", "0.25"],
        input=text)


def wav_seconds(path):
    with wave.open(str(path)) as w:
        return w.getnframes() / w.getframerate()


def split_phrases(text):
    """Sentences, then clauses at commas when a sentence is too long for two caption lines."""
    out = []
    for sent in re.split(r"(?<=[.!?])\s+", text.strip()):
        out.extend(split_long(sent))
    return [p for p in out if p]


def split_long(sent):
    if len(sent) <= MAX_CAPTION_CHARS:
        return [sent]
    parts, cur = [], ""
    for clause in re.split(r"(?<=[,;:])\s+", sent):
        if cur and len(cur) + len(clause) + 1 > MAX_CAPTION_CHARS:
            parts.append(cur)
            cur = clause
        else:
            cur = f"{cur} {clause}".strip()
    return parts + [cur]


def caption_timings(text, dur):
    phrases = split_phrases(text)
    total = sum(len(p) for p in phrases) or 1
    t, caps = 0.0, []
    for p in phrases:
        d = dur * len(p) / total
        caps.append({"start": round(t, 3), "end": round(t + d, 3), "text": p})
        t += d
    return caps


def narrate(script_path, out):
    script = load_yaml(script_path)
    model = script.get("voice", MODEL)
    out.mkdir(parents=True, exist_ok=True)
    scenes = []
    for sc in script["scenes"]:
        wav = out / f"{sc['id']}.wav"
        synth(sc["text"], model, wav)
        dur = wav_seconds(wav)
        scenes.append({"id": sc["id"], "wav": str(wav), "duration": round(dur, 3),
                       "text": sc["text"], "captions": caption_timings(sc["text"], dur)})
        print(f"{sc['id']}: {dur:.1f}s")
    (out / "timings.json").write_text(json.dumps({"scenes": scenes}, indent=1))
    print(f"total {sum(s['duration'] for s in scenes):.1f}s -> {out / 'timings.json'}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("script")
    ap.add_argument("--out")
    a = ap.parse_args()
    out = Path(a.out) if a.out else WORK / "narration" / Path(a.script).stem
    narrate(a.script, out)


if __name__ == "__main__":
    main()
