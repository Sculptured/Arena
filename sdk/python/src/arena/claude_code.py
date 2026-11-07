"""Run a headless Claude Code CLI (``claude -p``) as one of your external agents.

The CLI gets only the agent's MCP endpoint (the world's tools and the talk tools): built-in tools off,
settings sources empty, strict MCP config, as arenad's own CLI mode runs it. Every model call the CLI
reports is recorded on the branch (``report_usage``) and the agent's end with its tokens and cost.
It needs ``claude`` on PATH and signed in; it uses real model calls and costs credits.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
import time
from dataclasses import dataclass, field
from typing import Any, Optional

from .client import AgentEndpoint
from .errors import ArenaError
from .models import Task

#: Environment variables the CLI never gets (database URLs, other vendors' keys).
SCRUB = ("PGPASSWORD", "PGUSER", "PGHOST", "PGDATABASE", "PGPORT", "ARENA_PG_URL", "DATABASE_URL",
         "OPENROUTER_API_KEY", "OPENAI_API_KEY", "ARENA_LOCAL_MODELS")


class ClaudeCodeMissing(ArenaError):
    """``claude`` is not on PATH."""


@dataclass
class ClaudeResult:
    ok: bool
    text: str
    error: Optional[str]
    prompt_tokens: int
    completion_tokens: int
    cost_usd: Optional[float]
    seconds: float
    turns: int
    raw: list[dict[str, Any]] = field(default_factory=list, repr=False)


def available() -> bool:
    return shutil.which("claude") is not None


def task_prompt(task: Task, role: str = "") -> str:
    """The input arenad's own runners give an agent: its role prompt, the ticket and the
    benchmark's instructions."""
    parts = [p for p in (role.strip(), f"# Ticket\n\n{task.ticket}", task.instructions) if p]
    return "\n\n".join(parts)


def command(agent: AgentEndpoint, prompt: str, model: str, config_path: str,
            max_turns: Optional[int] = None) -> list[str]:
    cmd = [
        "claude", "-p", "--model", model,
        "--output-format", "stream-json", "--verbose",
        "--setting-sources", "",
        "--strict-mcp-config", "--mcp-config", config_path,
        "--disable-slash-commands",
        "--tools", "",
        "--allowedTools", "mcp__arena__*",
        "--no-session-persistence",
    ]
    if max_turns:
        cmd += ["--max-turns", str(max_turns)]
    return cmd + ["--", prompt]


def run(
    agent: AgentEndpoint,
    prompt: str,
    model: str = "haiku",
    max_turns: Optional[int] = None,
    timeout: float = 1800.0,
    report: bool = True,
    end: bool = True,
) -> ClaudeResult:
    """Runs Claude Code as ``agent`` until it stops. With ``report``, each model call it makes is
    recorded on the branch; with ``end``, the agent's end is recorded with its totals."""
    if not available():
        raise ClaudeCodeMissing("Claude Code (`claude`) is not on PATH; install it and sign in")
    workdir = tempfile.mkdtemp(prefix="arena-claude-")
    cfg = os.path.join(workdir, "mcp.json")
    with open(cfg, "w", encoding="utf-8") as f:
        json.dump(agent.mcp_config("arena"), f)
    env = {k: v for k, v in os.environ.items() if k not in SCRUB}
    started = time.monotonic()
    lines: list[dict[str, Any]] = []
    seen: set[str] = set()
    final: dict[str, Any] = {}
    try:
        proc = subprocess.Popen(
            command(agent, prompt, model, cfg, max_turns),
            cwd=workdir, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            stdin=subprocess.DEVNULL, text=True,
        )
        assert proc.stdout is not None
        for line in proc.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            lines.append(msg)
            if msg.get("type") == "assistant" and report:
                m = msg.get("message") or {}
                mid = m.get("id")
                if mid and mid not in seen and m.get("usage"):
                    seen.add(mid)
                    u = m["usage"]
                    agent.report_usage(
                        message_id=mid,
                        model=m.get("model"),
                        input_tokens=u.get("input_tokens"),
                        output_tokens=u.get("output_tokens"),
                        cache_creation_input_tokens=u.get("cache_creation_input_tokens"),
                        cache_read_input_tokens=u.get("cache_read_input_tokens"),
                    )
            elif msg.get("type") == "result":
                final = msg
            if time.monotonic() - started > timeout:
                proc.kill()
                break
        proc.wait(30)
        stderr = proc.stderr.read() if proc.stderr else ""
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
    seconds = time.monotonic() - started
    u = final.get("usage") or {}
    prompt_tokens = sum(int(u.get(k) or 0) for k in
                        ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))
    completion = int(u.get("output_tokens") or 0)
    is_error = bool(final.get("is_error")) or not final
    text = final.get("result") or ""
    error = None
    if is_error:
        error = text or (stderr.strip()[-500:] if stderr else "") or "claude ended without a result"
    res = ClaudeResult(
        ok=not is_error, text=text, error=error, prompt_tokens=prompt_tokens,
        completion_tokens=completion, cost_usd=final.get("total_cost_usd"), seconds=seconds,
        turns=int(final.get("num_turns") or 0), raw=lines,
    )
    if end:
        agent.end(
            ok=res.ok, error=res.error, prompt_tokens=prompt_tokens, completion_tokens=completion,
            cost_usd=res.cost_usd, cost_source="api_equivalent_estimate", seconds=round(seconds, 3),
        )
    return res
