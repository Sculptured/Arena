"""The engine's routing and status rules, on the designs' data: no model, no agent, no Arena.
How the engine drives real agents (mail to idle agents, wakes, reports, the end, a usage limit)
is only exercised by a real run (`run.py`, Claude Code agents on a running Arena); no test stands
in for an agent. Run: python -m unittest test_engine (from this folder)."""
import unittest

from designs import design_a, design_b, design_c
from engine import Engine, status_of


def routing(design):
    """An engine asked only who gets a message: its roles have no agent behind them."""
    return Engine(design, "b", {r.id: None for r in design.roles}, {}, lambda rec: None)


class EngineTest(unittest.TestCase):
    def test_a_topic_reaches_its_subscribers_only(self):
        e = routing(design_b())
        self.assertEqual(e.recipients("maintenance", "*", "#diagnosis.ready PR-A1 outer race"),
                         ["procurement", "planning", "comms"])
        self.assertEqual(e.recipients("comms", "*", "#approval.requested c-1"), ["planning"])
        self.assertEqual(e.recipients("planning", "*", "#unknown x"), [])

    def test_a_direct_message_reaches_its_agent_and_a_broadcast_everyone_else(self):
        e = routing(design_a())
        self.assertEqual(e.recipients("commander", "procurement", "go"), ["procurement"])
        self.assertEqual(e.recipients("commander", "nobody", "go"), [])
        self.assertEqual(len(e.recipients("commander", "*", "all hands")), 4)

    def test_the_status_line(self):
        self.assertEqual(status_of("did it\nSTATUS: DONE"), "DONE")
        self.assertEqual(status_of("no line"), "WAITING")
        self.assertEqual(status_of("STATUS: done\nlater STATUS: waiting"), "WAITING")

    def test_every_design_gives_its_roles_the_same_status_rule(self):
        for d in (design_a(), design_b(), design_c()):
            for r in d.roles:
                self.assertIn("STATUS: DONE", r.prompt)


if __name__ == "__main__":
    unittest.main()
