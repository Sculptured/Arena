"""Bring your own agent: your code drives the agents, Shakedown gives each one the world's tools (an MCP
endpoint) and a model gateway (OpenAI-compatible), and records everything.

    uv run python examples/bring_your_own_agent.py [task_id] [claude-code|openai|openrouter|anthropic] [model]

- ``claude-code`` (default): Claude Code CLI pointed at the agent's MCP URL (``arena.claude_code``).
- ``openai`` / ``openrouter`` / ``anthropic``: a minimal tool-calling loop written here, speaking
  OpenAI chat completions to the agent's gateway, which forwards to the provider with arenad's key and
  records every call. Needs the provider's key in arenad's environment; skips otherwise.

Real model calls: it costs credits.
"""

from __future__ import annotations

import json
import sys
import time

import httpx

from arena import Agent, claude_code
from common import connect, task_arg

via = sys.argv[2] if len(sys.argv) > 2 else "claude-code"
DEFAULT_MODELS = {"claude-code": "haiku", "openai": "gpt-4.1-mini", "openrouter": "deepseek/deepseek-chat-v3.1",
                  "anthropic": "claude-haiku-4-5"}
model = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_MODELS.get(via, "")


def gateway_loop(agent, prompt: str, model: str, max_turns: int = 30) -> None:
    """A minimal agent: ask the model, run the tool calls it makes through the MCP endpoint, repeat."""
    tools = [t.openai_schema() for t in agent.tools]
    messages = [{"role": "user", "content": prompt}]
    with httpx.Client(base_url=agent.gateway_url, timeout=300) as llm:
        for _ in range(max_turns):
            # The gateway holds the key; the client sends none.
            r = llm.post("/chat/completions", json={"model": model, "messages": messages, "tools": tools})
            r.raise_for_status()
            msg = r.json()["choices"][0]["message"]
            messages.append(msg)
            if msg.get("content"):
                print(f"  model: {msg['content'][:200]}")
            calls = msg.get("tool_calls") or []
            if not calls:
                break
            for call in calls:
                args = json.loads(call["function"].get("arguments") or "{}")
                res = agent.call(call["function"]["name"], args)
                print(f"  {call['function']['name']} -> {'error' if res.is_error else 'ok'}")
                messages.append({"role": "tool", "tool_call_id": call["id"], "content": res.text})
    agent.end(ok=True)


with connect() as arena:
    task = arena.task(task_arg(arena))
    if via == "claude-code":
        if not claude_code.available():
            sys.exit("skipped: `claude` is not on PATH; install Claude Code and sign in")
        spec = Agent.external("agent", model=model)
    else:
        status = {p["id"]: p for p in arena.providers()}.get(via)
        if not status or not status.get("available"):
            why = (status or {}).get("why", f"unknown provider {via}")
            sys.exit(f"skipped: the {via} provider is not available in this arenad ({why}); "
                     "set its key in arenad's environment")
        spec = Agent.external("agent", model=model, provider=via)

    branch = arena.external_run(task, agents=[spec], seed=1)
    agent = branch.agents["agent"]
    print(f"branch {branch.id}\n  MCP endpoint: {agent.mcp_url}\n  gateway:      {agent.gateway_url}")
    prompt = claude_code.task_prompt(task, "You are a customer-service agent. Resolve the ticket using the tools.")
    started = time.monotonic()
    if via == "claude-code":
        res = claude_code.run(agent, prompt, model=model)   # records its model calls and end
        print(f"  claude: ok={res.ok} turns={res.turns} tokens={res.prompt_tokens}+{res.completion_tokens}")
    else:
        gateway_loop(agent, prompt, model)
    print(f"  agent done in {time.monotonic() - started:.0f} s")

    result = branch.finish()                                 # freeze, score, stop the world
    check = branch.check()
    print(f"\n{result['status']}" + (f": {result['error']}" if result.get("error") else ""))
    for c in check.checks if check else []:
        print(f"  {'PASS' if c.passed else 'FAIL'} {c.name}")
    for e in branch.record().tool_calls():
        print(" ", e.summary())
