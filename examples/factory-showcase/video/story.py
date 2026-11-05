"""The narrator's sentences, computed from metrics.json: numbers and comparisons alike.

Rankings use each design's mean over its runs (`spread`); a design with one run says so.
Numbers are written the way the voice should say them ("15.5 million", "10 dollars 52").
"""

CHECK_TEXT = {
    "diagnosis_outer_race": "a diagnosis naming the outer race",
    "qualified_technician_dispatched": "sending a qualified technician",
    "correct_part_ordered_and_approved": "ordering the right part, approved",
    "purchase_order_in_erp": "the purchase order in the E R P",
    "customers_notified": "notifying every affected customer",
    "affected_orders_noticed": "covering every affected order",
    "crm_accounts_updated": "updating every C R M account",
    "crm_close_dates_moved": "moving the C R M close dates",
}


def check_text(name: str) -> str:
    return CHECK_TEXT.get(name, name.replace("_", " "))


def say_tokens(n: float) -> str:
    return f"{n / 1e6:.1f} million" if n >= 1e6 else f"{n / 1e3:.0f} thousand"


def say_money(usd: float) -> str:
    return f"{int(usd)} dollars {round((usd - int(usd)) * 100):02d}"


def say_minutes(s: float) -> str:
    m = s / 60
    return f"{m:.0f} minutes" if m >= 2 else f"{s:.0f} seconds"


def mean(xs: list) -> float:
    return sum(xs) / len(xs) if xs else 0.0


class Story:
    def __init__(self, m: dict):
        self.m, self.d = m, m["designs"]
        self.keys = list(self.d)
        self.limit = m.get("context_limit")

    # ---- per design numbers
    def label(self, k):
        """The name on screen ("A, hierarchical")."""
        return self.d[k].get("label", k)

    def name(self, k):
        """The name the narrator says ("design A"): a label reads badly aloud."""
        return f"design {k}"

    def runs(self, k):
        return self.d[k]["runs"]

    def n(self, k):
        return len(self.runs(k))

    def checks(self, k):
        return mean([r["checks"]["passed"] for r in self.runs(k)])

    def total(self, k):
        return self.runs(k)[0]["checks"]["total"]

    def est(self, k):
        return mean([r["estimated_s"] for r in self.runs(k)])

    def cost(self, k):
        return mean([(r.get("cost") or {}).get("usd") or 0.0 for r in self.runs(k)])

    def input_tokens(self, k):
        return mean([sum(a.get("prompt_tokens") or 0 for a in r["agents"]) for r in self.runs(k)])

    def peak(self, k):
        """The largest context any of the design's agents was sent, per run (mean over runs)."""
        return mean([max((r.get("peak_context") or {}).values(), default=0) for r in self.runs(k)])

    def compactions(self, k):
        def count(v):
            return len(v) if isinstance(v, list) else int(v or 0)
        return mean([sum(count(v) for v in (r.get("compactions") or {}).values()) for r in self.runs(k)])

    def single(self):
        return next((k for k in self.keys if len(self.runs(k)[0]["agents"]) == 1), None)

    def missed(self, k):
        """Failed checks, most often failed first."""
        counts = {}
        for r in self.runs(k):
            for c in r["checks"].get("failed", []):
                counts[c] = counts.get(c, 0) + 1
        return sorted(counts, key=lambda c: -counts[c])

    # ---- sentences
    def over(self, k):
        n = self.n(k)
        return "In its one run so far" if n == 1 else f"Over {n} runs"

    def design_result(self, k):
        miss = self.missed(k)
        text = (f"{self.over(k)}, it passed {self.checks(k):g} of {self.total(k)} checks, and finished "
                f"in about {say_minutes(self.est(k))}.")
        if miss:
            text += f" It fell short on {', and '.join(check_text(c) for c in miss[:2])}."
        return text

    def lower_third(self, k):
        return (f"{self.checks(k):g}/{self.total(k)} checks · {self.est(k) / 60:.0f} min · "
                f"${self.cost(k):.2f} · {self.n(k)} run{'s' if self.n(k) > 1 else ''}")

    def step_line(self, k):
        if len(self.runs(k)[0]["agents"]) > 1:
            return ("Every message between agents is a recorded step, with who sent it, what it said, "
                    "and what the receiver did next.")
        return ("Every tool call is a recorded step, with its arguments and its full result, so you can "
                "see exactly what the agent had in front of it when it acted.")

    def ranked(self):
        return sorted(self.keys, key=lambda k: (-self.checks(k), self.est(k)))

    def verdict(self):
        best = self.ranked()[0]
        top = [k for k in self.keys if self.checks(k) == self.checks(best)]
        tot = self.total(best)
        if len(top) == 1:
            others = ", ".join(f"{self.checks(k):g} for {self.name(k)}" for k in self.keys if k != best)
            text = f"Arena scores every run with the same checks. {self.name(best).capitalize()} passed the most: " \
                   f"{self.checks(best):g} of {tot}, against {others}."
        else:
            names = " and ".join(self.name(k) for k in top).capitalize()
            fast = min(top, key=self.est)
            text = f"Arena scores every run with the same checks. {names} tied, with " \
                   f"{self.checks(best):g} of {tot}; {self.name(fast)} got there faster."
        if self.checks(best) < tot:
            text += " No design passed every check."
        if min(self.n(k) for k in self.keys) < 2:
            text += " Some designs have only one run so far, so treat this as a first look."
        return text

    def speed_line(self):
        order = sorted(self.keys, key=self.est)
        parts = ", ".join(f"{self.name(k)} in {say_minutes(self.est(k))}" for k in order)
        return f"On time, measured from the alarm and net of Arena's own overhead: {parts}."

    def context_line(self):
        c = self.single()
        if not c or not self.limit:
            return "Each agent's context is recorded at every model call."
        pk, lim = self.peak(c), self.limit
        if self.compactions(c) > 0 or pk >= 0.9 * lim:
            text = (f"The single agent's context reached {say_tokens(pk)} tokens of its "
                    f"{say_tokens(lim)} window, and it had to compact, dropping earlier detail.")
        else:
            text = (f"The single agent's context peaked at {say_tokens(pk)} tokens, "
                    f"{100 * pk / lim:.0f} percent of its {say_tokens(lim)} window: in this world it "
                    f"never ran out of room.")
        return text + self.specialist_line(c, pk)

    def specialist_line(self, c, pk):
        multi = [k for k in self.keys if k != c]
        top = max((self.peak(k) for k in multi), default=0)
        if not multi:
            return ""
        if top > pk:
            return (f" The busiest specialists in the multi-agent designs carried even more, up to "
                    f"{say_tokens(top)}, because every message they received stayed in their context.")
        return (f" In the multi-agent designs each specialist carried only its part: at most "
                f"{say_tokens(top)}.")

    def roles_line(self):
        parts = []
        for k in self.keys:
            r = self.runs(k)[0]
            msgs = sum(x["count"] for x in r.get("messages") or [])
            topics = r.get("topics") or {}
            if topics:
                parts.append(f"in {self.name(k)}, {sum(topics.values())} events went out on "
                             f"{len(topics)} topics")
            elif msgs:
                parts.append(f"in {self.name(k)}, the agents exchanged {msgs} messages")
        if not parts:
            return "Arena records every message between agents."
        return "Splitting the work has a price in coordination: " + "; ".join(parts) + "."

    def cost_line(self):
        k = self.keys[0]
        r = self.runs(k)[0]
        costs = ", ".join(f"{self.name(k)} {say_money(self.cost(k))}" for k in self.keys)
        return (f"Arena times its own work, such as saving and restoring the world, and takes it out: "
                f"{self.name(k).capitalize()}'s first run took {r['measured_s']:.0f} seconds, {r['overhead_s']:.0f} "
                f"of them Arena's, so about {r['estimated_s']:.0f} in production. Per run, at the A P I "
                f"prices Claude Code reports: {costs}.")

    def outro(self):
        return ("Arena didn't choose the design. It ran each one as it was built, recorded every call, "
                "and scored the outcome, so the choice rests on measurements, not on a diagram.")
