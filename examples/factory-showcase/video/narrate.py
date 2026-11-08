#!/usr/bin/env python3
"""Narration: render measured speech beats, intentional pauses, and timings.json.

    uv run narrate.py script.yaml [--out DIR]

script.yaml:  voice: <optional model path>;  scenes: [{id, text}, ...]
Writes DIR/<id>.wav and DIR/timings.json (default DIR: WORK/narration/<script name>).
Caption timing within each beat is estimated; inter-beat pauses use measured WAV durations.
backend: kokoro selects the natural-voice path; legacy scripts default to Piper.
"""
import argparse
import json
import re
import wave
import textwrap
from pathlib import Path

from common import MODEL, WORK, load_yaml
from voice import make_voice

MAX_CAPTION_CHARS = 84


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
    return [line for part in parts + [cur] for line in textwrap.wrap(
        part, MAX_CAPTION_CHARS, break_long_words=True, break_on_hyphens=False)]


def render_beats(sc, speak, out, pause):
    """Measure each spoken beat; captions never stretch across intentional silence."""
    beats = sc.get("beats") or [{"text": t} for t in re.split(r"(?<=[.!?])\s+", sc["text"].strip())]
    wav = out / f"{sc['id']}.wav"
    caps, elapsed = [], 0.0
    with wave.open(str(wav), "wb") as joined:
        for i, beat in enumerate(beats):
            raw = out / f"{sc['id']}-{i:02d}.wav"
            speak(beat.get("spoken", beat["text"]), raw)
            with wave.open(str(raw), "rb") as source:
                params = source.getparams()
                if i == 0:
                    joined.setparams(params)
                elif (params.nchannels, params.sampwidth, params.framerate) != (
                        joined.getnchannels(), joined.getsampwidth(), joined.getframerate()):
                    raise ValueError("Narration beats must use the same PCM audio format")
                joined.writeframes(source.readframes(params.nframes))
            duration = params.nframes / params.framerate
            caps.extend({**c, "start": round(c["start"] + elapsed, 3),
                         "end": round(c["end"] + elapsed, 3)}
                        for c in caption_timings(beat["text"], duration))
            gap = float(beat.get("pause_after", pause)) if i < len(beats) - 1 else 0
            if gap < 0:
                raise ValueError("pause_after must be nonnegative")
            frames = round(gap * params.framerate)
            joined.writeframes(b"\0" * frames * params.sampwidth * params.nchannels)
            elapsed += duration + frames / params.framerate
    return wav, caps


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
    speak = make_voice(script, model)
    out.mkdir(parents=True, exist_ok=True)
    scenes = []
    for sc in script["scenes"]:
        if not re.fullmatch(r"[A-Za-z0-9_-]+", sc["id"]):
            raise ValueError("Scene ids must contain only letters, numbers, underscores and hyphens")
        wav, caps = render_beats(sc, speak, out, script.get("pause", 0.5))
        dur = wav_seconds(wav)
        scenes.append({"id": sc["id"], "wav": str(wav), "duration": round(dur, 3),
                       "text": sc["text"], "captions": caps})
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
