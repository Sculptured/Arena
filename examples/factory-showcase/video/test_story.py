"""Offline tests of the narration's sentences (no Arena, no voice)."""
import unittest

from story import Story


def run(passed, est, peak, agents=("a", "b"), compactions=None, failed=()):
    return {"checks": {"passed": passed, "total": 12, "failed": list(failed)}, "estimated_s": est,
            "measured_s": est + 10, "overhead_s": 10, "cost": {"usd": 1.5},
            "peak_context": {a: peak for a in agents}, "compactions": compactions or {},
            "agents": [{"agent": a, "prompt_tokens": 1000} for a in agents], "messages": [], "topics": {}}


def metrics(**designs):
    return {"context_limit": 200000, "designs": {k: {"label": k, "runs": r} for k, r in designs.items()}}


class StoryTest(unittest.TestCase):
    def test_the_design_with_most_checks_wins_and_says_the_others(self):
        s = Story(metrics(A=[run(7, 900, 1), run(9, 700, 1)], C=[run(6, 400, 1, agents=("x",))] * 2))
        self.assertIn("Design A passed the most: 8 of 12, against 6 for design C.", s.verdict())
        self.assertIn("No design passed every check.", s.verdict())

    def test_a_tie_names_the_faster_and_one_run_is_a_first_look(self):
        s = Story(metrics(B=[run(9, 660, 1)], C=[run(9, 420, 1, agents=("x",))]))
        v = s.verdict()
        self.assertIn("Design B and design C tied, with 9 of 12; design C got there faster", v)
        self.assertIn("first look", v)

    def test_context_says_room_left_or_compaction_as_recorded(self):
        roomy = Story(metrics(A=[run(9, 1, 166000)], C=[run(9, 1, 126000, agents=("x",))]))
        self.assertIn("63 percent of its 200 thousand window", roomy.context_line())
        self.assertIn("carried even more, up to 166 thousand", roomy.context_line())
        tight = Story(metrics(A=[run(9, 1, 90000)],
                              C=[run(5, 1, 190000, agents=("x",), compactions={"x": 2})]))
        self.assertIn("it had to compact", tight.context_line())
        self.assertIn("at most 90 thousand", tight.context_line())

    def test_a_design_says_what_it_missed_most_often(self):
        s = Story(metrics(A=[run(9, 600, 1, failed=["customers_notified", "x_y"]),
                             run(9, 600, 1, failed=["customers_notified"])]))
        self.assertIn("It fell short on notifying every affected customer, and x y.", s.design_result("A"))


if __name__ == "__main__":
    unittest.main()
