"""Voice engines. Kokoro is explicit: never silently substitute the old draft voice."""
import sys

from common import run


def make_voice(script, model):
    backend = script.get("backend", "piper")
    if backend == "piper":
        def speak(text, path):
            run([sys.executable, "-m", "piper", "-m", model, "-f", path,
                 "--sentence-silence", "0.32"], input=text)
        return speak
    if backend != "kokoro":
        raise ValueError(f"Unknown narration backend: {backend}")
    try:
        import numpy as np
        import soundfile as sf
        from kokoro import KPipeline
    except ImportError as exc:
        raise RuntimeError("Kokoro voice unavailable. Install requirements-natural.txt with Python 3.12, "
                           "and cache the model and voice before rendering offline. "
                           "The Piper draft voice will not be substituted.") from exc
    pipeline = KPipeline(lang_code=script.get("language", "a"))

    def speak(text, path):
        parts = [audio for _, _, audio in pipeline(
            text, voice=script.get("speaker", "af_heart"), speed=script.get("speed", 0.96))]
        if not parts:
            raise RuntimeError("Voice engine returned no audio")
        sf.write(str(path), np.concatenate(parts), 24000, subtype="PCM_16")
    return speak
