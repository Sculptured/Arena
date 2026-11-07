"""Integration tests against a real arenad (see conftest.py). They check the API contracts the SDK
relies on by driving the API directly; no agent, scripted or otherwise, takes part."""

from __future__ import annotations

import json
import os
import threading

import httpx
import pytest

from arena import BadRequest, NotFound, NotSupported
from arena.env import END, ArenaParallelEnv
from arena.export import read_jsonl

pytestmark = pytest.mark.core


def _read_tool(branch, agent) -> str:
    """A read tool of the world (calling it with no arguments is recorded, error or not)."""
    effects = branch.record().run_start.get("tools", {})
    names = [t.name for t in agent.tools if effects.get(t.name, {}).get("effect") == "read"]
    assert names, "the world has no read tool"
    return names[0]


def test_core_describes_itself(arena):
    h = arena.health()
    assert "benchmark" in h and "providers" in h
    b = arena.benchmark()
    assert b.name and b.tasks
    tasks = arena.tasks()
    assert tasks[0].ticket and tasks[0].instructions
    assert arena.task(tasks[0].id).id == tasks[0].id
    assert arena.default_team(model="haiku").agents
    assert all(a.model == "haiku" for a in arena.default_team(model="haiku").agents)
    assert arena.world()
    assert isinstance(arena.supports_pause(), bool)


def test_bad_requests_are_typed(arena):
    with pytest.raises((NotFound, BadRequest)):
        arena.external_run("no-such-task")
    with pytest.raises(BadRequest):
        arena.external_run(arena.tasks()[0].id, seed=2**60)
    with pytest.raises(NotFound):
        arena.branch("no-such-branch")


def test_external_run_record_fork_compare_export(arena, cleanup, tmp_path):
    task = arena.tasks()[0]
    branch = arena.external_run(task, agents=["probe"], seed=7, name="sdk test")
    cleanup.append(branch.run_id)
    agent = branch.agents["probe"]
    assert agent.mcp_url.endswith(f"/mcp/{branch.id}/probe") and "/gateway/" in agent.gateway_url
    assert agent.mcp_config()["mcpServers"]["arena"]["url"] == agent.mcp_url
    names = {t.name for t in agent.tools}
    assert {"handoff", "message", "inbox"} <= names
    assert {t.name for t in agent.mcp.list_tools()} == names

    tool = _read_tool(branch, agent)
    result = agent.call(tool, {})
    assert isinstance(result.text, str)
    agent.end(ok=True)
    rec = branch.record()
    assert rec.status == "running" and rec.seed == 7 and rec.run_start["runner"] == "external"
    calls = rec.tool_calls("probe")
    assert [c.tool for c in calls] == [tool]
    assert any(r.id == branch.run_id and r.name == "sdk test" for r in arena.runs())

    ended = branch.finish()
    assert ended["status"] in ("passed", "failed")
    check = branch.check()
    assert check is not None and check.checks

    # The stream of an ended branch gives every event, then the final status.
    seen = list(branch.stream())
    assert [e.seq for e in seen] == [e.seq for e in branch.events()]
    assert branch.final["status"] == ended["status"]

    # Fork at the tool call: the world restored, the new branch live with the agent's history.
    fork = branch.fork(calls[0].seq)
    cleanup.append(fork.run_id)
    assert fork.fork_info.start_seq == calls[0].seq
    assert "probe" in fork.agents
    assert all(e.seq < calls[0].seq for e in fork.agents["probe"].history)
    assert fork.record().status == "running"
    plan = branch.restore_plan(calls[0].seq)
    assert plan["parts"]
    fork.finish()
    cmp = arena.compare(branch, fork)
    assert "first_divergent_seq" in cmp

    out = branch.export(str(tmp_path / "ds"))
    rows = read_jsonl(os.path.join(out, "branches.jsonl"))
    events = read_jsonl(os.path.join(out, "events.jsonl"))
    assert rows[0]["branch_id"] == branch.id and rows[0]["seed"] == 7
    assert rows[0]["events"] == len(events) and events[0]["kind"] == "run_start"
    assert "arena-dataset/1" in open(os.path.join(out, "SCHEMA.md")).read()


def test_live_stream_follows_a_running_branch(arena, cleanup):
    branch = arena.external_run(arena.tasks()[0], agents=["probe"])
    cleanup.append(branch.run_id)
    got: list = []
    t = threading.Thread(target=lambda: got.extend(branch.stream()))
    t.start()
    agent = branch.agents["probe"]
    agent.call(_read_tool(branch, agent), {})
    agent.end()
    branch.finish()
    t.join(60)
    assert not t.is_alive()
    kinds = [e.kind for e in got]
    assert kinds[0] == "run_start" and "tool_call" in kinds and kinds[-1] == "check"
    assert [e.seq for e in got] == sorted(e.seq for e in got)
    assert branch.final["status"] in ("passed", "failed")


def test_pause_is_feature_detected(arena, cleanup):
    branch = arena.external_run(arena.tasks()[0], agents=["probe"])
    cleanup.append(branch.run_id)
    try:
        if arena.supports_pause():
            assert branch.pause().get("paused") is True
            assert branch.record().paused is True
            assert branch.resume().get("paused") is False
        else:
            with pytest.raises(NotSupported):
                branch.pause()
    finally:
        branch.finish()


def test_cors_origin_is_configurable(arena):
    if not hasattr(arena, "process"):
        pytest.skip("only for an arenad the tests started")
    ok = httpx.options(f"{arena.url}/api/runs", headers={
        "origin": "http://127.0.0.1:5173", "access-control-request-method": "POST"})
    assert ok.status_code == 204
    assert ok.headers["access-control-allow-origin"] == "http://127.0.0.1:5173"
    other = httpx.get(f"{arena.url}/api/health", headers={"origin": "http://evil.example"})
    assert "access-control-allow-origin" not in other.headers


def test_step_env_contract(arena, cleanup):
    env = ArenaParallelEnv(arena, arena.tasks()[0].id, agents=["probe"], max_steps=5)
    obs, infos = env.reset(seed=11)
    cleanup.append(env.branch.run_id)
    assert env.agents == ["probe"] and infos["probe"]["seed"] == 11
    assert obs["probe"]["ticket"] and obs["probe"]["tools"]
    tool = _read_tool(env.branch, env.branch.agents["probe"])
    obs, rewards, terms, truncs, infos = env.step({"probe": {"name": tool, "arguments": {}}})
    assert obs["probe"]["tool_result"] is not None and env.agents == ["probe"]
    assert rewards == {"probe": 0.0} and not terms["probe"] and not truncs["probe"]
    obs, rewards, terms, truncs, infos = env.step({"probe": END})
    assert env.agents == [] and terms["probe"] and infos["probe"]["check"] is not None
    assert rewards["probe"] in (0.0, 1.0)
    calls = env.branch.record().tool_calls("probe")
    env2, obs2, infos2 = env.fork(calls[0].seq)
    cleanup.append(env2.branch.run_id)
    assert env2.agents == ["probe"] and "history" in obs2["probe"]
    env2.close()
    assert env2.branch.record().status in ("passed", "failed")


def test_external_suite_claim_and_export(arena, cleanup, tmp_path):
    task = arena.tasks()[0].id
    suite = arena.suite(team=[], tasks=[task], repeats=1, runner="external", name="sdk suite")
    branch = suite.claim()
    assert branch is not None and branch.task_id == task
    cleanup.append(branch.run_id)
    branch.register("probe").end()
    branch.finish()
    assert suite.claim() is None
    report = suite.refresh()
    assert report["summary"]["runs"] == 1 and report["summary"]["ended"] == 1
    assert suite.compare(suite)["verdict"] in ("same", "incomparable")
    out = suite.export(str(tmp_path / "suite"))
    assert json.load(open(os.path.join(out, "suite.json")))["id"] == suite.id
    assert len(read_jsonl(os.path.join(out, "branches.jsonl"))) == 1
