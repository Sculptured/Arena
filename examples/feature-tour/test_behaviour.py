"""Offline: python3 -m unittest discover -s examples/feature-tour -v."""
import unittest
from behaviour import BASE_PLAN, TASKS, behaviour


class BehaviourTest(unittest.TestCase):
    def test_exactly_one_v1_safety_mistake_and_three_refusals(self):
        rows = [behaviour("v1", task, repeat) for task in TASKS for repeat in (1, 2)]
        self.assertEqual(sum(row.short_dated for row in rows), 1)
        self.assertEqual(sum(row.premature_write for row in rows), 3)
        self.assertEqual(rows[1].plan[0][1], "CVX-2589")

    def test_tuning_removes_both_mistakes(self):
        for task in TASKS:
            for repeat in (1, 2):
                row = behaviour("v2", task, repeat)
                self.assertFalse(row.short_dated or row.premature_write)
                self.assertEqual(row, behaviour("v2", task, repeat))

    def test_short_scenario_defers_two_orders_and_never_releases(self):
        for deployment in ("v1", "v2", "single"):
            row = behaviour(deployment, TASKS[1], 1)
            self.assertFalse(row.release)
            self.assertEqual(len(row.plan), 6)
            self.assertFalse(any(lot == "CVX-2608" for _, lot, _ in row.plan))
            deferred = {po for po, _, _ in BASE_PLAN} - {po for po, _, _ in row.plan}
            self.assertEqual(deferred, {"PO-PWD-7719", "PO-MSD-88147"})

    def test_single_spends_more_time_and_reads_without_safety_mistakes(self):
        for task in TASKS:
            row = behaviour("single", task, 1)
            self.assertGreater(row.extra_reads, 0)
            self.assertGreater(row.delay, behaviour("v2", task, 1).delay)
            self.assertFalse(row.short_dated or row.premature_write)

    def test_invalid_inputs_fail_closed(self):
        for args in (("bad", TASKS[0], 1), ("v1", "bad", 1), ("v1", TASKS[0], 0)):
            with self.assertRaises(ValueError):
                behaviour(*args)
