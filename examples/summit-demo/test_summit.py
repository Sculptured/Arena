"""Offline checks of the summit demo driver's decisions; no arenad, no model."""
import json
import unittest

from calls import Divergence, Recorded, ResumingCalls, recorded_calls
from decisions import BASE_PLAN, allocation, behaviour, deployment_of
from drive import scenario_arg
from incident import choose
from teams import V2_RULES, fork_body, team, with_rules

LANES = [{"carrier": "Tundra", "origin": "US", "qualified": True},
         {"carrier": "Borealis", "origin": "US", "qualified": True},
         {"carrier": "Northgate", "origin": "US", "qualified": False},
         {"carrier": "Glacier", "origin": "EU", "qualified": True, "available": False}]


def quote(carrier, arrives, price, capacity=5, temperature="2-8 C", origin="US"):
    return {"carrier": carrier, "arrives": arrives, "price": price, "capacity_left": capacity,
            "temperature": temperature, "origin": origin}


class Decisions(unittest.TestCase):
    def test_v1_keeps_its_mistakes_and_v2_and_the_single_agent_avoid_them(self):
        live = behaviour("v1", "live")
        self.assertTrue(live.premature_write and live.short_dated and not live.checks_shipments)
        self.assertEqual([behaviour("v1", "batch", r).premature_write for r in (1, 2)], [True, False])
        self.assertEqual([behaviour("v1", "batch", r).short_dated for r in (1, 2)], [False, True])
        for d in ("v2", "single"):
            b = behaviour(d, "live")
            self.assertFalse(b.premature_write or b.short_dated)
            self.assertTrue(b.checks_shipments)
        self.assertTrue(behaviour("v1", "duplicate").expedite)
        with self.assertRaises(ValueError):
            behaviour("v3", "live")

    def test_a_failed_retest_moves_the_next_lots_orders_to_the_eu_lot(self):
        plan = allocation(behaviour("v2", "batch"), released_next=False)
        self.assertNotIn("CVX-2608", {lot for _, lot, _ in plan})
        self.assertEqual(len(plan), len(BASE_PLAN) - 2, "the EU orders wait for the restock")
        short = allocation(behaviour("v1", "live"), released_next=True)
        self.assertEqual(short[0][1], "CVX-2589")

    def test_the_deployment_of_a_team(self):
        self.assertEqual(deployment_of(team("single")), "single")
        self.assertEqual(deployment_of(team("v1")), "v1")
        swapped = {"id": "summit-v1", "agents": [{**a, "prompt": with_rules(a["prompt"], V2_RULES)}
                                                 for a in team("v1")["agents"]]}
        self.assertEqual(deployment_of(swapped), "v2")
        coordinator = team("v1", expediting=True)["agents"][0]
        self.assertFalse(coordinator["read_only"])


class Lanes(unittest.TestCase):
    def test_the_cheapest_on_time_lane_else_the_earliest_on_a_qualified_available_cold_lane(self):
        quotes = [quote("Tundra", "2026-11-10", 380), quote("Borealis", "2026-11-11", 420),
                  quote("Northgate", "2026-11-10", 95, temperature="15-25 C")]
        self.assertEqual(choose(quotes, LANES, set(), "2026-11-11")["carrier"], "Tundra")
        full = [quote("Tundra", "2026-11-10", 380, capacity=0), quote("Borealis", "2026-11-11", 420)]
        self.assertEqual(choose(full, LANES, set(), "2026-11-10")["carrier"], "Borealis", "the next lane")
        self.assertIsNone(choose(full, LANES, {"Borealis"}, "2026-11-10"))
        delayed = [quote("Tundra", "2026-11-12", 380), quote("Borealis", "2026-11-11", 420)]
        self.assertEqual(choose(delayed, LANES, set(), "2026-11-10")["carrier"], "Borealis")
        eu = [quote("Glacier", "2026-11-11", 3400, origin="EU")]
        self.assertIsNone(choose(eu, LANES, set(), "2026-11-12"), "an unavailable lane is no choice")


class Resume(unittest.TestCase):
    def resuming(self, record):
        c = ResumingCalls.__new__(ResumingCalls)
        c.record, c.branch = record, "b" * 32
        c.actor = lambda role: role
        c.live = []
        c.paced, c.pace = False, 0.0
        return c

    def test_the_record_answers_until_its_writes_are_used_then_calls_are_live(self):
        record = [Recorded("affected_orders", {}, False, json.dumps({"orders": []})),
                  Recorded("propose_change", {"action": "deviation"}, False, '{"id": "c1"}'),
                  Recorded("book_shipment", {"proposal_id": "x"}, True, "refused")]
        c = self.resuming(record)
        self.assertEqual(c.take("affected_orders", {}).text, '{"orders": []}')
        self.assertTrue(c.replaying())
        self.assertIsNone(c.take("propose_change", {"action": "hold"}))
        with self.assertRaises(Divergence):
            c.do("quality", "insert_doc", {"doc": {}})
        self.assertEqual(c.do("quality", "propose_change", {"action": "deviation"}), (False, '{"id": "c1"}'))
        self.assertFalse(c.replaying(), "a refused write changed nothing, so it holds nothing up")

    def test_only_the_agents_calls_before_the_fork_point_are_the_record(self):
        events = [{"seq": 2, "kind": "tool_call", "agent": "setup", "payload": {"name": "s"}},
                  {"seq": 3, "kind": "tool_call", "agent": "a", "payload": {"name": "t", "args": {}, "result": 1}},
                  {"seq": 4, "kind": "tool_call", "agent": "a", "payload": {"name": "u", "error": "no"}},
                  {"seq": 5, "kind": "tool_call", "agent": "a", "payload": {"name": "late"}}]
        got = [(r.name, r.refused) for r in recorded_calls(events, 5)]
        self.assertEqual(got, [("t", False), ("u", True)])


class Requests(unittest.TestCase):
    def test_a_disruption_argument_and_the_fork_that_swaps_the_team(self):
        self.assertEqual(scenario_arg("transit-delay:carrier=Tundra Medical Courier,days=2"),
                         {"disruptions": [{"id": "transit-delay",
                                           "params": {"carrier": "Tundra Medical Courier", "days": 2}}]})
        self.assertIsNone(scenario_arg(""))
        parent = team("v1")
        self.assertEqual(fork_body(parent, 9, "single")["team"]["agents"][0]["id"], "operator")
        v2 = fork_body(parent, 9, "v2")
        self.assertEqual(set(v2["changes"]), {a["id"] for a in parent["agents"]})
        self.assertIn("Deployment rules:", v2["changes"]["logistics"]["prompt"])


if __name__ == "__main__":
    unittest.main()
