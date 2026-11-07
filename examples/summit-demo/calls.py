"""Tool calls of the scripted agents, on the feature tour's client: a call that may be refused,
pacing by phase (fast up to the shipments, slow over the first one, then medium), and resuming a
fork, where the calls before the fork point are answered from the record instead of made again."""
import json
import sys
import time
from dataclasses import dataclass
from pathlib import Path

# Appended, so this folder's own modules (drive.py) win over the feature tour's of the same name.
sys.path.append(str(Path(__file__).resolve().parents[1] / "feature-tour"))
from client import TEAMS, Calls  # noqa: E402  (the feature tour's client: usage, ends, tool ownership)

# The world's tools that change it. A fork's recorded prefix must be used up, in order, before the
# new deployment acts on its own: these are what the record holds the new behaviour to.
WRITES = frozenset({"insert_doc", "update_doc", "crm_log_note", "crm_update_opportunity", "cold_chain_quotes",
                    "book_shipment", "send_customer_notice", "propose_change"})
TALK = frozenset({"message", "inbox", "handoff"})
# Seconds between calls per phase, for a paced run: `start` until the shipments, `focus` over the
# first shipment (where the disruption and the refusal show), `rest` after it.
PACES = {"start": 0.1, "focus": 1.0, "rest": 0.05}


class DemoCalls(Calls):
    def __init__(self, api, started, behaviour, paced):
        super().__init__(api, started, behaviour.deployment, 0.0, behaviour)
        self.paced = paced
        self.phase("start")

    def phase(self, name):
        """Moves to a pacing phase (no pause at all when the run isn't paced)."""
        self.pace = PACES[name] if self.paced else 0.0

    def check_tool(self, agent, tool):
        """The design's tool ownership (data/teams.json), as the feature tour's client checks it."""
        allowed = {t for group in self.roles[agent]["tools"] for t in TEAMS["tools"][group]}
        if tool not in allowed | TALK:
            raise ValueError(f"{agent} cannot use {tool}")

    def do(self, role, tool, args):
        """Makes the call through the agent's MCP endpoint: (refused, text)."""
        agent = self.actor(role)
        self.check_tool(agent, tool)
        time.sleep(self.pace)
        n = self.counts.get(agent, 0) + 1
        self.counts[agent] = n
        self.usage(agent, n)
        body = {"jsonrpc": "2.0", "id": n, "method": "tools/call", "params": {"name": tool, "arguments": args}}
        result = self.api.request(f"/mcp/{self.branch}/{agent}", body)
        if "error" in result:
            raise RuntimeError(result["error"])
        out = result["result"]
        refused = bool(out.get("isError"))
        print(f"{self.branch[:8]} {agent}: {tool}" + (" [refused]" if refused else ""), flush=True)
        return refused, out["content"][0]["text"]

    def call(self, role, tool, args=None, refused=False):
        got, text = self.do(role, tool, args or {})
        if got != refused:
            raise RuntimeError(f"{self.actor(role)}/{tool}: expected refusal={refused}: {text}")
        return text if refused else json.loads(text)

    def attempt(self, role, tool, args=None):
        """A call the world may refuse: (True, result) or (False, why)."""
        refused, text = self.do(role, tool, args or {})
        return (False, text) if refused else (True, json.loads(text))


@dataclass
class Recorded:
    name: str
    args: dict
    refused: bool
    text: str
    used: bool = False

    @property
    def is_write(self):
        return self.name in WRITES and not self.refused


def recorded_calls(events, before_seq):
    """The agents' tool calls before the fork point, as they were answered."""
    out = []
    for e in events:
        if e["seq"] >= before_seq or e["kind"] != "tool_call" or e["agent"] in (None, "link", "setup"):
            continue
        p = e["payload"]
        refused = p.get("error") is not None
        text = p["error"] if refused else json.dumps(p.get("result"))
        out.append(Recorded(p["name"], p.get("args") or {}, refused, text))
    return out


class Divergence(RuntimeError):
    pass


class ResumingCalls(DemoCalls):
    """A fork resumed: the new deployment's calls are answered from the record while the record still
    holds writes it hasn't used (the world already has them); a read it adds is made live, a write
    the record lacks there is a divergence. Once every recorded write is used, calls are live."""

    def __init__(self, api, started, behaviour, paced, record):
        super().__init__(api, started, behaviour, paced)
        self.record = record

    def replaying(self):
        return any(r.is_write and not r.used for r in self.record)

    def take(self, tool, args):
        """The first unused recorded call with this tool and these args; a write must also be the
        next unused write (the world has them in that order)."""
        hit = next((r for r in self.record if not r.used and r.name == tool and r.args == args), None)
        if hit and hit.is_write:
            first = next(r for r in self.record if r.is_write and not r.used)
            hit = hit if first is hit else None
        if hit:
            hit.used = True
        return hit

    def do(self, role, tool, args):
        if not self.replaying():
            return super().do(role, tool, args)
        hit = self.take(tool, args)
        if hit:
            print(f"{self.branch[:8]} {self.actor(role)}: {tool} (from the record)", flush=True)
            return hit.refused, hit.text
        if tool in WRITES:
            raise Divergence(f"{tool} {json.dumps(args)} is not where the record has its next write: "
                             "this deployment acts differently before the fork point; fork earlier")
        return super().do(role, tool, args)
