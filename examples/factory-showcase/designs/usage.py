"""Counts one Claude Code agent's model calls from its stream-json and reports each to Arena.

Two things in that stream need care:
- Claude Code repeats an assistant message once per content block (same message id, same usage),
  so a call is reported once, by its id (Arena also replaces a call reported again).
- The `output_tokens` of a streamed assistant message is a placeholder (1, or a few); the real
  count is in the turn's `result` usage. The turn's output is given to its calls by re-reporting the
  turn's last call with the output the other calls don't account for.
- Some calls are never streamed: compacting the context is a model call of its own. The `result`
  carries the session's cumulative `modelUsage`, so what the streamed calls don't account for is
  reported as one more call (`unstreamed-<turn>`), and the totals match what Claude Code billed.
"""


def call_report(message: dict, context_window: int | None) -> dict:
    u = message.get("usage") or {}
    keys = ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens", "output_tokens")
    report = {k: int(u.get(k) or 0) for k in keys}
    report.update(message_id=message.get("id"), model=message.get("model"),
                  context_window=context_window)
    return report


class UsageMeter:
    """`report(dict)` sends one usage report to Arena. `prompt_tokens` and `completion_tokens`
    are the agent's totals so far, over distinct calls."""

    def __init__(self, report, context_window: int | None):
        self.report, self.context_window = report, context_window
        self.prompt_tokens = self.completion_tokens = 0
        self._seen = set()
        self._turn = []  # this turn's call reports, in order
        self._turns = 0

    def assistant(self, message: dict) -> None:
        r = call_report(message, self.context_window)
        if r["message_id"] in self._seen:
            return
        self._seen.add(r["message_id"])
        self._turn.append(r)
        self.prompt_tokens += r["input_tokens"] + r["cache_creation_input_tokens"] + r["cache_read_input_tokens"]
        self.completion_tokens += r["output_tokens"]
        self.report(r)

    def turn_ended(self, result: dict) -> None:
        """The turn's `result`: its real output tokens go to its calls (see the module's notes)."""
        calls, self._turn = self._turn, []
        self._turns += 1
        out = (result.get("usage") or {}).get("output_tokens")
        if calls and out is not None:
            self._give_output(calls, int(out))
        self._report_unstreamed(result.get("modelUsage") or {})

    def _give_output(self, calls: list, out: int) -> None:
        last = calls[-1]
        rest = sum(c["output_tokens"] for c in calls[:-1])
        corrected = max(out - rest, 0)
        if corrected != last["output_tokens"]:
            self.completion_tokens += corrected - last["output_tokens"]
            self.report(dict(last, output_tokens=corrected))

    def _report_unstreamed(self, model_usage: dict) -> None:
        """The session's billed tokens (cumulative, all models) beyond the streamed calls' sums."""
        def total(*keys):
            return sum(int(u.get(k) or 0) for u in model_usage.values() for k in keys)
        gap_in = total("inputTokens", "cacheReadInputTokens", "cacheCreationInputTokens") - self.prompt_tokens
        gap_out = total("outputTokens") - self.completion_tokens
        if not model_usage or (gap_in <= 0 and gap_out <= 0):
            return
        gap_in, gap_out = max(gap_in, 0), max(gap_out, 0)
        self.prompt_tokens += gap_in
        self.completion_tokens += gap_out
        self.report({"message_id": f"unstreamed-{self._turns}", "model": next(iter(model_usage)),
                     "input_tokens": gap_in, "cache_creation_input_tokens": 0,
                     "cache_read_input_tokens": 0, "output_tokens": gap_out,
                     "context_window": self.context_window})
