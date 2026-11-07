#!/usr/bin/env python3
"""Narration with Kokoro (Apache-2.0, offline): one WAV per scene and timings.json, like narrate.py.

    <venv with kokoro, soundfile, pyyaml>/python narrate_kokoro.py script.yaml --out DIR

script.yaml: `voice` (a Kokoro voice, default af_heart), `speed` (default 1.0) and
scenes [{id, text, say?}]. `text` is the caption; `say`, when given, is what is spoken (for
spelled-out acronyms). Each sentence is synthesised on its own and joined with a short pause, so
caption times come from the measured length of each sentence, not an estimate.
"""
import argparse
import json
import re
from pathlib import Path

import numpy as np
import soundfile as sf
import yaml
from kokoro import KPipeline

RATE = 24000
PAUSE_S = 0.32
MAX_CAPTION_CHARS = 84


def sentences(text):
    return [s for s in re.split(r"(?<=[.!?])\s+", text.strip()) if s]


def clauses(sentence):
    """A sentence too long for two caption lines, split at commas, colons and semicolons."""
    if len(sentence) <= MAX_CAPTION_CHARS:
        return [sentence]
    parts, cur = [], ""
    for c in re.split(r"(?<=[,;:])\s+", sentence):
        if cur and len(cur) + len(c) + 1 > MAX_CAPTION_CHARS:
            parts.append(cur)
            cur = c
        else:
            cur = f"{cur} {c}".strip()
    return parts + [cur]


def synth(pipe, voice, speed, text):
    chunks = [np.asarray(a, dtype=np.float32) for _, _, a in pipe(text, voice=voice, speed=speed)]
    return np.concatenate(chunks) if chunks else np.zeros(0, dtype=np.float32)


def caption_spans(sentence, start, length):
    """The sentence's caption phrases over [start, start + length], split by phrase length."""
    phrases = clauses(sentence)
    total = sum(len(p) for p in phrases) or 1
    out, t = [], start
    for p in phrases:
        d = length * len(p) / total
        out.append({"start": round(t, 3), "end": round(t + d, 3), "text": p})
        t += d
    return out


def scene_audio(pipe, voice, speed, sc):
    """The scene's samples and captions. Spoken and caption sentences pair up one to one."""
    shown, spoken = sentences(sc["text"]), sentences(sc.get("say", sc["text"]))
    if len(shown) != len(spoken):
        raise ValueError(f"{sc['id']}: `say` and `text` need the same number of sentences")
    pause = np.zeros(int(PAUSE_S * RATE), dtype=np.float32)
    parts, caps, t = [], [], 0.0
    for i, (cap, say) in enumerate(zip(shown, spoken)):
        audio = synth(pipe, voice, speed, say)
        caps += caption_spans(cap, t, len(audio) / RATE)
        parts.append(audio)
        t += len(audio) / RATE
        if i < len(shown) - 1:
            parts.append(pause)
            t += PAUSE_S
    return np.concatenate(parts), caps


def narrate(script_path, out):
    script = yaml.safe_load(Path(script_path).read_text())
    voice, speed = script.get("voice", "af_heart"), float(script.get("speed", 1.0))
    pipe = KPipeline(lang_code=voice[0])
    out.mkdir(parents=True, exist_ok=True)
    scenes = []
    for sc in script["scenes"]:
        audio, caps = scene_audio(pipe, voice, speed, sc)
        wav = out / f"{sc['id']}.wav"
        sf.write(wav, audio, RATE)
        dur = len(audio) / RATE
        scenes.append({"id": sc["id"], "wav": str(wav), "duration": round(dur, 3),
                       "text": sc["text"], "captions": caps})
        print(f"{sc['id']}: {dur:.1f}s", flush=True)
    (out / "timings.json").write_text(json.dumps({"scenes": scenes}, indent=1))
    print(f"total {sum(s['duration'] for s in scenes):.1f}s -> {out / 'timings.json'}")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("script")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()
    narrate(a.script, Path(a.out))


if __name__ == "__main__":
    main()
