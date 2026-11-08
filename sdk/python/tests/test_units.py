"""Unit tests: parsing, models, errors, export rows. No arenad needed."""

import json

import pytest

from arena import Agent, Change, Disruption, Scenario, Team
from arena.errors import BadRequest, Conflict, DiskFull, NotFound, from_status, ApiError
from arena.export import branch_row, event_rows
from arena.models import BranchRecord, CheckResult, Event, as_scenario, as_team
from arena.sse import parse
from arena.mcp import Tool, ToolResult
from arena.env import _messages_for, _parse_action, END
from arena.client import wait_review


def test_sse_parse_events_comments_and_multiline():
    lines = [
        ": keep-alive",
        "event: event",
        'data: {"seq": 1, "kind": "run_start"}',
        "",
        "event: status",
        "data: {",
        'data: "status": "passed"}',
        "",
        "data: tail",
    ]
    got = list(parse(lines))
    assert got == [("event", {"seq": 1, "kind": "run_start"}), ("status", {"status": "passed"}),
                   ("message", "tail")]


def test_agent_and_team_dicts():
    a = Agent.api("solver", provider="openai", model="gpt-x", prompt="p", read_only=True)
    d = a.to_dict()
    assert d == {"id": "solver", "name": "solver", "kind": "open", "model": "gpt-x", "prompt": "p",
                 "read_only": True, "provider": "openai"}
    assert Agent.from_dict(d) == Agent("solver", "open", "gpt-x", "p", "solver", "openai", True)
    t = Team(agents=[Agent.claude_code("a"), Agent.codex("b")]).with_all(model="haiku")
    assert [x.model for x in t.agents] == ["haiku", "haiku"]
    assert as_team([Agent.external("x")])["agents"][0]["kind"] == "external"
    assert as_team(None) == {"id": "team", "name": "Team", "agents": []}


def test_scenarios_and_changes():
    assert as_scenario(["lane", Disruption("cap", {"after": 2})]) == {
        "disruptions": [{"id": "lane"}, {"id": "cap", "params": {"after": 2}}]
    }
    assert as_scenario(Scenario([Disruption("x")])) == {"disruptions": [{"id": "x"}]}
    assert as_scenario(None) is None
    assert Change(model="", prompt="new").to_dict() == {"model": "", "prompt": "new"}


def test_check_result_named_and_gold():
    c = CheckResult.from_payload({"passed": False, "checks": [
        {"name": "released", "passed": True},
        {"name": "shipped", "passed": False, "detail": "no shipment"},
        {"name": "no_unapproved_write", "passed": False, "safety": True},
    ], "differing_rows": ["shipped"]})
    assert c.score == 0.5
    assert [v.name for v in c.safety_violations] == ["no_unapproved_write"]
    g = CheckResult.from_payload({"passed": True, "differing_rows": []})
    assert [x.name for x in g.checks] == ["matches_gold"] and g.score == 1.0


def test_errors_by_status():
    assert isinstance(from_status(400, "x"), BadRequest)
    assert isinstance(from_status(404, "x"), NotFound)
    assert isinstance(from_status(409, "x"), Conflict)
    assert isinstance(from_status(507, "x"), DiskFull)
    e = from_status(500, "boom", "GET", "/api/x")
    assert type(e) is ApiError and e.status == 500 and "GET /api/x" in str(e)


def _record(start_seq=None):
    return BranchRecord.from_dict({
        "id": "b2", "run_id": "r1", "status": "failed", "error": None, "parent_id": "b1" if start_seq else None,
        "start_seq": start_seq, "team": {"id": "t", "name": "t", "agents": [{"id": "a", "kind": "external"}]},
        "created_at": "2026-11-07T23:17:50.759274Z", "ended_at": "2026-11-07T23:17:58Z",
        "restore_mode": "exact" if start_seq else None, "restored": [], "fork_note": None,
        "metrics": {"measured_s": 7.8},
        "events": [
            {"branch_id": "b2", "seq": 1, "at": "2026-11-07T23:17:51Z", "agent": None, "kind": "run_start",
             "payload": {"task_id": "72", "seed": 42, "benchmark": "retail", "runner": "external"}},
            {"branch_id": "b2", "seq": 2, "at": "2026-11-07T23:17:52Z", "agent": "a", "kind": "tool_call",
             "payload": {"name": "get_user_details", "args": {"user_id": "u"}, "error": "User not found",
                         "writes": []}},
            {"branch_id": "b2", "seq": 3, "at": "2026-11-07T23:17:57Z", "agent": None, "kind": "check",
             "payload": {"passed": False, "differing_rows": ["orders/#W1"]}},
            {"branch_id": "b2", "seq": 4, "at": "2026-11-07T23:17:58Z", "agent": None, "kind": "pause",
             "payload": {}},
        ],
    })


def test_branch_record_and_export_rows():
    rec = _record(start_seq=3)
    assert rec.seed == 42 and rec.ended and rec.check.differing_rows == ["orders/#W1"]
    assert rec.tool_calls("a")[0].tool == "get_user_details"
    row = branch_row(rec)
    assert row["format"] == "arena-dataset/1" and row["task_id"] == "72" and row["passed"] is False
    assert row["checks"] == [{"name": "matches_gold", "passed": False, "detail": "", "safety": False}]
    rows = list(event_rows(rec))
    assert [r["inherited"] for r in rows] == [True, True, False, False]
    json.dumps(rows)  # serialisable
    assert "get_user_details" in rec.events[1].summary()
    assert rec.events[3].summary().strip().endswith("pause")  # unknown kinds are kept


def test_tools_and_results():
    t = Tool.from_dict({"name": "n", "description": "d", "inputSchema": {"type": "object"}})
    assert t.openai_schema()["function"]["parameters"] == {"type": "object"}
    assert t.anthropic_schema()["input_schema"] == {"type": "object"}
    assert ToolResult('{"a": 1}', False).value == {"a": 1}
    assert ToolResult("plain", True).value == "plain"


def test_env_helpers():
    assert _parse_action({"name": "x", "arguments": {"a": 1}}) == ("x", {"a": 1})
    assert _parse_action(("x", None)) == ("x", {})
    with pytest.raises(ValueError):
        _parse_action("x")
    assert END == "__end__"
    ev = [Event("b", 5, None, "a", "message", {"to": "b", "text": "hi"}),
          Event("b", 6, None, "c", "message", {"to": "*", "text": "all"}),
          Event("b", 7, None, "a", "handoff", {"note": "n"}),
          Event("b", 8, None, "b", "message", {"to": "*", "text": "own"})]
    assert [m["text"] for m in _messages_for("b", ev)] == ["hi", "all", "n"]


def test_wait_review_follows_a_running_review_to_its_end():
    answers = iter([{"status": "running"}, {"status": "done", "review": {"verdict": "v"}}])
    slept = []
    r = wait_review(lambda: next(answers), {"status": "running", "key": "branch:b"}, 60, 1.5, slept.append)
    assert r["review"] == {"verdict": "v"} and slept == [1.5, 1.5]
    assert wait_review(lambda: None, {"status": "done"}, 60, 1, slept.append) == {"status": "done"}
    with pytest.raises(TimeoutError):
        wait_review(lambda: {"status": "running"}, {"status": "running"}, -1, 0, lambda _: None)
