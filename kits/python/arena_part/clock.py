"""Fixed-step clock: the way to meet the clock contract (state after advance is split-independent)."""


class FixedStep:
    def __init__(self, step_ms: int = 100):
        if step_ms <= 0:
            raise ValueError("step_ms must be positive")
        self.step_ms = step_ms
        self.done_ms = 0

    def advance_to(self, clock_ms: int, fn) -> int:
        """Call `fn(step_ms)` once per whole step up to `clock_ms`; the remainder waits for the next
        call. Returns how many steps ran."""
        n = 0
        while self.done_ms + self.step_ms <= clock_ms:
            self.done_ms += self.step_ms
            fn(self.step_ms)
            n += 1
        return n
