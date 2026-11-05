"""One agent as a long-lived Claude Code process: headless (`claude -p`), with the same isolation
flags as arena's own driver (core/crates/arenad/src/agents/claude.rs), but taking its input as a
stream of user messages, so a design can give it more work without losing its context. Each model
call its stream reports (cache tokens included; see usage.py) and each compaction is reported to
Arena, which records them as `model_call` events."""
import json
import os
import subprocess
import tempfile
import threading
import time

import arena_api
from usage import UsageMeter

# The variables arena's driver removes from a CLI's environment (agents/mod.rs, scrub_env).
SCRUBBED = ("ARENA_PG_URL", "DATABASE_URL", "OPENROUTER_API_KEY", "ARENA_LOCAL_MODELS", "OPENAI_API_KEY")


def scrubbed_env() -> dict:
    env = {k: v for k, v in os.environ.items()
           if k not in SCRUBBED and not k.startswith(("PG", "ARENA_LOCAL_KEY_"))}
    return env


def command(model: str, server: str, mcp_url: str) -> list:
    config = json.dumps({"mcpServers": {server: {"type": "http", "url": mcp_url}}})
    return ["claude", "-p", "--model", model, "--input-format", "stream-json",
            "--output-format", "stream-json", "--verbose", "--setting-sources", "",
            "--strict-mcp-config", "--mcp-config", config, "--disable-slash-commands",
            "--tools", "", "--allowedTools", f"mcp__{server}__*", "--no-session-persistence"]


class Agent:
    """`busy` while it works on a turn; `last_text` is what it said at the end of its last turn."""

    def __init__(self, branch: str, agent_id: str, model: str, mcp_url: str, out_dir: str,
                 context_window: int | None, server: str = "factory"):
        self.branch, self.id, self.model = branch, agent_id, model
        self.context_window = context_window
        self.busy = False
        self.last_text = ""
        self.turns = 0
        self.completed = 0  # turns ended so far
        self.mcp_url = mcp_url
        self.cost_usd = None
        self.error = None
        self.started = time.monotonic()
        self.usage = UsageMeter(lambda r: arena_api.report_usage(branch, agent_id, r), context_window)
        self._closing = False
        self._lock = threading.Lock()
        self._log = open(os.path.join(out_dir, f"{agent_id}.jsonl"), "w")
        self._err = open(os.path.join(out_dir, f"{agent_id}.stderr"), "w")
        self.workdir = tempfile.mkdtemp(prefix=f"arena-showcase-{agent_id}-")
        self.proc = subprocess.Popen(command(model, server, mcp_url), stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=self._err, text=True,
                                     cwd=self.workdir, env=scrubbed_env(), bufsize=1)
        self._reader = threading.Thread(target=self._read, daemon=True)
        self._reader.start()

    def send(self, text: str) -> None:
        """Starts a turn with `text` as the user's message."""
        with self._lock:
            self.busy = True
            self.turns += 1
        msg = {"type": "user", "message": {"role": "user", "content": text}}
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def _read(self) -> None:
        for line in self.proc.stdout:
            self._log.write(line)
            self._log.flush()
            try:
                self._on(json.loads(line))
            except (ValueError, arena_api.ArenaError) as e:
                self._err.write(f"showcase: {e}\n")
        code = self.proc.wait()
        with self._lock:
            self.busy = False
            if not self._closing and self.error is None:
                self.error = f"claude exited ({code}) in the middle of the run"

    def _on(self, d: dict) -> None:
        kind = d.get("type")
        if kind == "assistant":
            self.usage.assistant(d["message"])
        elif kind == "system" and d.get("subtype") == "compact_boundary":
            arena_api.report_usage(self.branch, self.id, {"compaction": d.get("compact_metadata", {})})
        elif kind == "result":
            self.usage.turn_ended(d)
            self._ended_turn(d)

    def _ended_turn(self, d: dict) -> None:
        with self._lock:
            self.last_text = d.get("result") or ""
            # Claude Code's total_cost_usd is the session's so far (cumulative across turns).
            self.cost_usd = d.get("total_cost_usd", self.cost_usd)
            if d.get("is_error"):
                self.error = self.last_text or d.get("subtype") or "claude reported an error"
            self.completed += 1
            self.busy = False

    def end(self) -> dict:
        """Stops the process and reports the agent's end to Arena: its tokens and cost."""
        self._closing = True
        if self.busy:  # the run is over (out of time): its turn is cut off
            self.error = self.error or "stopped in the middle of a turn when the run ended"
            self.proc.kill()
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
        self._reader.join(timeout=10)
        done = {"ok": self.error is None, "error": self.error,
                "prompt_tokens": self.usage.prompt_tokens, "completion_tokens": self.usage.completion_tokens,
                "cost_usd": self.cost_usd, "cost_source": "api_equivalent_estimate",
                "seconds": round(time.monotonic() - self.started, 1)}
        arena_api.end_agent(self.branch, self.id, done)
        return done
