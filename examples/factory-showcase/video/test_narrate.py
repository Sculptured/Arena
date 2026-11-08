"""Offline timing checks; no voice model is called."""
import tempfile
import unittest
import wave
from pathlib import Path

from narrate import render_beats, split_phrases, wav_seconds
from story import say_money


def silent_voice(text, path):
    with wave.open(str(path), "wb") as wav:
        wav.setnchannels(1)
        wav.setsampwidth(2)
        wav.setframerate(24000)
        wav.writeframes(b"\0" * 24000 * 2)


class NarrationTiming(unittest.TestCase):
    def test_explicit_pause_is_outside_captions(self):
        scene = {"id": "intro", "text": "First. Second.", "beats": [
            {"text": "First.", "pause_after": 0.75}, {"text": "Second."}]}
        with tempfile.TemporaryDirectory() as out:
            wav, caps = render_beats(scene, silent_voice, Path(out), 0.5)
            self.assertEqual(wav_seconds(wav), 2.75)
            self.assertEqual(caps, [{"start": 0.0, "end": 1.0, "text": "First."},
                                    {"start": 1.75, "end": 2.75, "text": "Second."}])

    def test_unpunctuated_text_stays_readable(self):
        text = "technical detail " * 30
        phrases = split_phrases(text)
        self.assertTrue(all(len(p) <= 84 for p in phrases))
        self.assertEqual(" ".join(phrases), text.strip())

    def test_money_rounds_before_splitting_dollars_and_cents(self):
        self.assertEqual(say_money(1.999), "2 dollars and 0 cents")
        self.assertEqual(say_money(1.01), "1 dollar and 1 cent")


if __name__ == "__main__":
    unittest.main()
