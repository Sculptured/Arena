"""Transport contract and role isolation, without a server or model."""
import unittest
from unittest.mock import Mock, patch

from behaviour import TASKS, behaviour
from client import Calls, deployment_team
from drive import audit_branch, suite_id
from scenario import next_work_order


class RecordingAPI:
    def __init__(self):
        self.requests = []

    def request(self, path, body=None):
        self.requests.append((path, body))
        return {"result": {"content": [{"type": "text", "text": '{"orders": []}'}], "isError": False}}


class ClientTest(unittest.TestCase):
    @patch("scenario.time.sleep")
    def test_missing_work_order_is_retried_through_recorded_calls(self, sleep):
        calls = Mock()
        calls.call.side_effect = [[], [{"name": "work"}]]
        self.assertEqual(next_work_order(calls), "work")
        self.assertEqual(calls.call.call_count, 2)
        sleep.assert_called_once_with(1)

    @patch("scenario.time.sleep")
    def test_missing_work_order_fails_after_bounded_reads(self, sleep):
        calls = Mock()
        calls.call.return_value = []
        with self.assertRaisesRegex(RuntimeError, "three recorded reads"):
            next_work_order(calls)
        self.assertEqual(calls.call.call_count, 3)
        self.assertEqual(sleep.call_count, 2)

    def test_resume_reuses_suite_and_leaves_completed_runs_alone(self):
        api = Mock()
        api.request.return_value = {"runs": [{"status": s} for s in ("passed", "failed", "pending")]}
        record = {"suites": {"v1": {"id": "existing", "runs": [{"branch_id": "kept"}]}}}
        self.assertEqual(suite_id(api, record, "v1", "team v1", 2), "existing")
        api.request.assert_called_once_with("/api/suites/existing")
        self.assertEqual(record["suites"]["v1"]["runs"], [{"branch_id": "kept"}])

    def test_resume_refuses_unfinished_or_lost_runs(self):
        for status in ("starting", "running", "error", "missing", "stopped"):
            with self.subTest(status=status):
                api = Mock()
                api.request.return_value = {"runs": [{"status": status}]}
                with self.assertRaisesRegex(RuntimeError, "Cannot resume"):
                    suite_id(api, {"suites": {"v1": {"id": "existing"}}}, "v1", "team v1", 2)
                api.request.assert_called_once_with("/api/suites/existing")

    def calls(self, deployment="v2"):
        api = RecordingAPI()
        return api, Calls(api, {"branch_id": "branch"}, deployment, 0,
                          behaviour(deployment, TASKS[0], 1))

    def test_all_agents_are_external(self):
        for deployment in ("v1", "v2", "single"):
            team = deployment_team(deployment)
            self.assertTrue(team["name"].startswith("Demo:"))
            self.assertTrue(all(a["kind"] == "external" for a in team["agents"]))

    def test_role_cannot_use_another_roles_tools(self):
        api, calls = self.calls()
        with self.assertRaises(ValueError):
            calls.call("quality", "list_docs", {})
        self.assertEqual(api.requests, [])

    @patch("client.time.sleep")
    def test_usage_is_per_call_grows_and_has_distinct_message_ids(self, sleep):
        api, calls = self.calls()
        for _ in range(2):
            calls.call("coordinator", "affected_orders")
        usage = [body for path, body in api.requests if path.endswith("/usage")]
        self.assertLess(usage[0]["input_tokens"], usage[1]["input_tokens"])
        self.assertNotEqual(usage[0]["message_id"], usage[1]["message_id"])
        self.assertFalse(any("gateway" in path for path, _ in api.requests))
        self.assertEqual(sleep.call_count, 2)

    def test_audit_counts_refused_writes_separately_from_reads(self):
        events = [{"kind": "tool_call", "payload": {"name": "stock_report"}},
                  {"kind": "tool_call", "payload": {"name": "book_shipment", "error": "refused"}},
                  {"kind": "message", "payload": {}}]
        self.assertEqual(audit_branch({"events": events}), {
            "tool_calls": 2, "read_calls": 1, "refused_bookings": 1, "messages": 1})
