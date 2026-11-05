"""The engine's routing, status and end rules, with fake agents: no model, no Arena.
Run: python -m unittest test_engine (from this folder)."""
import unittest

from designs import design_a, design_b, design_c
from engine import Engine, status_of


class FakeAgent:
    def __init__(self):
        self.busy, self.error, self.turns, self.last_text, self.sent = False, None, 0, "", []
        self.completed, self.mcp_url = 0, "http://mcp"

    def send(self, text):
        self.sent.append(text)
        self.turns += 1


def engine(design):
    agents = {r.id: FakeAgent() for r in design.roles}
    return Engine(design, "b", agents, {}, lambda rec: None), agents


class EngineTest(unittest.TestCase):
    def test_a_topic_reaches_its_subscribers_only(self):
        e, _ = engine(design_b())
        self.assertEqual(e.recipients("maintenance", "*", "#diagnosis.ready PR-A1 outer race"),
                         ["procurement", "planning", "comms"])
        self.assertEqual(e.recipients("comms", "*", "#approval.requested c-1"), ["planning"])
        self.assertEqual(e.recipients("planning", "*", "#unknown x"), [])

    def test_a_direct_message_reaches_its_agent_and_a_broadcast_everyone_else(self):
        e, _ = engine(design_a())
        self.assertEqual(e.recipients("commander", "procurement", "go"), ["procurement"])
        self.assertEqual(e.recipients("commander", "nobody", "go"), [])
        self.assertEqual(len(e.recipients("commander", "*", "all hands")), 4)

    def test_mail_waits_for_an_idle_agent_and_comes_as_one_turn(self):
        e, agents = engine(design_a())
        agents["procurement"].busy = True
        for seq in (5, 6):
            e.on_event({"kind": "message", "seq": seq, "agent": "commander",
                        "payload": {"to": "procurement", "text": f"m{seq}"}})
        self.assertFalse(e.deliver())
        agents["procurement"].busy = False
        self.assertTrue(e.deliver())
        self.assertEqual(len(agents["procurement"].sent), 1)
        self.assertIn("m5", agents["procurement"].sent[0])
        self.assertIn("m6", agents["procurement"].sent[0])

    def test_the_status_line_and_the_end_condition(self):
        self.assertEqual(status_of("did it\nSTATUS: DONE"), "DONE")
        self.assertEqual(status_of("no line"), "WAITING")
        e, agents = engine(design_c())
        agents["incident"].turns, agents["incident"].last_text = 1, "STATUS: WAITING"
        self.assertFalse(e.done())
        self.assertTrue(e.wake())
        agents["incident"].last_text = "all done\nSTATUS: DONE"
        self.assertTrue(e.done())

    def test_a_hierarchical_roles_turn_answer_goes_to_the_commander(self):
        import engine as eng
        calls = []
        eng.arena_api.mcp_call = lambda url, tool, args: calls.append((tool, args))
        e, agents = engine(design_a())
        agents["maintenance"].completed, agents["maintenance"].last_text = 1, "outer race"
        agents["commander"].completed, agents["commander"].last_text = 1, "summary"
        self.assertTrue(e.forward())
        self.assertEqual(calls, [("message", {"to": "commander",
                                              "text": "[report at the end of my turn]\nouter race"})])
        self.assertFalse(e.forward())
        e2, agents2 = engine(design_b())
        agents2["comms"].completed, agents2["comms"].last_text = 1, "done"
        self.assertFalse(e2.forward())

    def test_a_usage_limit_stops_the_run(self):
        e, agents = engine(design_c())
        self.assertFalse(e.usage_limited())
        agents["incident"].error = "You've hit your 5-hour limit"
        self.assertEqual(e.run(5), "usage limit")

    def test_every_design_gives_its_roles_the_same_status_rule(self):
        for d in (design_a(), design_b(), design_c()):
            for r in d.roles:
                self.assertIn("STATUS: DONE", r.prompt)


if __name__ == "__main__":
    unittest.main()
