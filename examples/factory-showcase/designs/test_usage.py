"""Offline tests of the usage meter (no model, no Arena)."""
import unittest

from usage import UsageMeter


def msg(mid, cache_read, out=1):
    return {"id": mid, "model": "m", "usage": {"input_tokens": 5, "cache_creation_input_tokens": 0,
                                               "cache_read_input_tokens": cache_read, "output_tokens": out}}


class UsageMeterTest(unittest.TestCase):
    def setUp(self):
        self.sent = []
        self.meter = UsageMeter(self.sent.append, 200_000)

    def test_a_message_repeated_per_content_block_is_one_call(self):
        for m in (msg("a", 100), msg("a", 100), msg("a", 100), msg("b", 300)):
            self.meter.assistant(m)
        self.assertEqual([r["message_id"] for r in self.sent], ["a", "b"])
        self.assertEqual(self.meter.prompt_tokens, 410)

    def test_the_turns_output_goes_to_its_last_call(self):
        self.meter.assistant(msg("a", 100))
        self.meter.assistant(msg("b", 300))
        self.meter.turn_ended({"usage": {"output_tokens": 900}})
        self.assertEqual(self.sent[-1]["message_id"], "b")
        self.assertEqual(self.sent[-1]["output_tokens"], 899)
        self.assertEqual(self.meter.completion_tokens, 900)

    def test_a_turn_without_calls_or_usage_reports_nothing_more(self):
        self.meter.turn_ended({"usage": {"output_tokens": 50}})
        self.meter.assistant(msg("a", 100))
        self.meter.turn_ended({})
        self.assertEqual(len(self.sent), 1)

    def test_billed_tokens_the_stream_never_showed_are_one_more_call(self):
        self.meter.assistant(msg("a", 100))
        billed = {"m": {"inputTokens": 5, "cacheReadInputTokens": 900, "cacheCreationInputTokens": 0,
                        "outputTokens": 70}}
        self.meter.turn_ended({"usage": {"output_tokens": 20}, "modelUsage": billed})
        extra = self.sent[-1]
        self.assertEqual((extra["message_id"], extra["input_tokens"], extra["output_tokens"]),
                         ("unstreamed-1", 800, 50))
        self.assertEqual((self.meter.prompt_tokens, self.meter.completion_tokens), (905, 70))
        self.meter.turn_ended({"modelUsage": billed})
        self.assertEqual(len(self.sent), 3)


if __name__ == "__main__":
    unittest.main()
