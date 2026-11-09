"""The step-loop environment with a real model as the policy.

    uv run python examples/step_env.py [task_id] [max_steps]

Each step the policy (a model) sees its observation and history and picks one tool call, or ends.
The policy is:

- the Claude Messages API, natively with tools, when ``ANTHROPIC_API_KEY`` is set here
  (``POLICY_MODEL``, default ``claude-haiku-4-5``); else
- Claude Code (``claude -p``, no tools of its own, JSON answer; ``POLICY_MODEL`` default ``haiku``).

Either way it costs credits. Shakedown supplies the world, the record and the reward, not the agent.
After the episode it forks at the first tool call ("reset to step N") and prints that the world is back.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys

import httpx

from arena.env import END, ArenaParallelEnv
from common import connect, task_arg

MAX_STEPS = int(sys.argv[2]) if len(sys.argv) > 2 else 12
ROLE = ("You are a customer-service agent acting in a store's systems through tools. "
        "Make one tool call per step. When the ticket is resolved, or nothing more can be done, end.")


class ClaudeApiPolicy:
    """Claude through the Messages API, the tools given natively."""

    def __init__(self, tools: list[dict], prompt: str):
        self.model = os.environ.get("POLICY_MODEL", "claude-haiku-4-5")
        self.tools = [{"name": t["name"], "description": t["description"], "input_schema": t["input_schema"]}
                      for t in tools] + [{"name": "end", "description": "End: the task is done.",
                                          "input_schema": {"type": "object", "properties": {}}}]
        self.messages: list[dict] = [{"role": "user", "content": prompt}]
        self.http = httpx.Client(base_url="https://api.anthropic.com/v1", timeout=120, headers={
            "x-api-key": os.environ["ANTHROPIC_API_KEY"], "anthropic-version": "2023-06-01"})
        self.pending: str | None = None

    def act(self, obs: dict):
        if self.pending is not None:
            res = obs["tool_result"] or {}
            self.messages.append({"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": self.pending, "content": res.get("text", ""),
                "is_error": bool(res.get("is_error"))}]})
        r = self.http.post("/messages", json={"model": self.model, "max_tokens": 1024,
                                              "system": ROLE, "tools": self.tools,
                                              "messages": self.messages})
        r.raise_for_status()
        content = r.json()["content"]
        self.messages.append({"role": "assistant", "content": content})
        use = next((c for c in content if c["type"] == "tool_use"), None)
        if use is None or use["name"] == "end":
            return END
        self.pending = use["id"]
        return {"name": use["name"], "arguments": use["input"]}


class ClaudeCodePolicy:
    """Claude Code as a stateless chooser: the whole history goes in each prompt, one JSON action out."""

    def __init__(self, tools: list[dict], prompt: str):
        self.model = os.environ.get("POLICY_MODEL", "haiku")
        self.catalog = "\n".join(
            f"- {t['name']}: {t['description']} args schema: {json.dumps(t['input_schema'])}" for t in tools)
        self.prompt = prompt
        self.history: list[str] = []
        self.cost = 0.0

    def act(self, obs: dict):
        if obs.get("tool_result") is not None:
            self.history[-1] += f"\n  result: {obs['tool_result']['text'][:1500]}"
        ask = (f"{self.prompt}\n\n# Tools\n{self.catalog}\n\n# What you did so far\n"
               + ("\n".join(self.history) or "(nothing yet)")
               + '\n\nAnswer with one JSON object only: {"name": "<tool>", "arguments": {...}} for the next '
                 'tool call, or {"end": true} when done.')
        out = subprocess.run(
            ["claude", "-p", "--model", self.model, "--output-format", "json", "--tools", "",
             "--setting-sources", "", "--strict-mcp-config", "--no-session-persistence",
             "--system-prompt", ROLE, "--", ask],
            capture_output=True, text=True, timeout=300, cwd="/tmp")
        reply = json.loads(out.stdout)
        self.cost += reply.get("total_cost_usd") or 0.0
        m = re.search(r"\{.*\}", reply.get("result", ""), re.S)
        action = json.loads(m.group(0)) if m else {"end": True}
        if action.get("end"):
            return END
        self.history.append(f"- called {action['name']}({json.dumps(action.get('arguments', {}))})")
        return {"name": action["name"], "arguments": action.get("arguments", {})}


def make_policy(tools, prompt):
    if os.environ.get("ANTHROPIC_API_KEY"):
        return ClaudeApiPolicy(tools, prompt)
    if shutil.which("claude"):
        return ClaudeCodePolicy(tools, prompt)
    sys.exit("skipped: no policy model: set ANTHROPIC_API_KEY, or install Claude Code (`claude`) and sign in")


with connect() as arena:
    task = task_arg(arena)
    env = ArenaParallelEnv(arena, task, agents=["support"], max_steps=MAX_STEPS, reward="score")
    obs, infos = env.reset(seed=3)
    tools = [t for t in obs["support"]["tools"] if t["name"] not in ("handoff", "message", "inbox")]
    policy = make_policy(tools, obs["support"]["prompt"])
    print(f"task {task}, branch {env.branch.id}, policy {type(policy).__name__} ({policy.model})")
    total = 0.0
    while env.agents:
        action = policy.act(obs["support"])
        obs, rewards, terms, truncs, infos = env.step({"support": action})
        total += rewards["support"]
        shown = "END" if action == END else f"{action['name']}({json.dumps(action['arguments'])[:80]})"
        res = obs["support"]["tool_result"]
        print(f"step {env.steps:>2}: {shown} -> {'' if res is None else ('error' if res['is_error'] else 'ok')}")
    check = infos["support"]["check"]
    print(f"\nepisode over: {infos['support']['status']}, reward {total}"
          + (" (truncated)" if truncs["support"] else ""))
    if check:
        print("differs from the expected world:", check.differing_rows)
    if isinstance(policy, ClaudeCodePolicy):
        print(f"policy cost (Claude Code's API-equivalent estimate): ${policy.cost:.4f}")

    calls = env.branch.record().tool_calls("support")
    if calls:
        env2, obs2, _ = env.fork(calls[0].seq)          # reset to step N: the world as it was then
        print(f"\nforked at step {calls[0].seq}: branch {env2.branch.id}, live, "
              f"{len(obs2['support']['history'])} events of history; closing it unchanged")
        env2.close()
