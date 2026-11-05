"""Drives a design's agents on one Arena branch. Arena's event stream says when an agent sends a
message or a note; the design's routing decides who gets it, and an idle agent gets its waiting mail
as its next turn. When every agent is idle with no mail, the run is over if the design's end
condition holds (its agents said `STATUS: DONE`); otherwise the agents still waiting are woken after
a pause, since the plant's clock keeps running (quotes, the trip, the shipment)."""
import json
import re
import threading
import time

import arena_api
from claude_agent import Agent

QUIET_S = 3  # how long the team must stay idle before it counts as idle (events may lag)
WAKE_AFTER_S = 30  # how long an idle team waits before its waiting agents are woken
MAX_WAKES = 20  # wakes per agent before the run is called stalled
STATUS_RE = re.compile(r"STATUS:\s*(DONE|WAITING)", re.I)


def status_of(text: str) -> str:
    found = STATUS_RE.findall(text or "")
    return found[-1].upper() if found else "WAITING"


class Engine:
    def __init__(self, design, branch: str, agents: dict, first: dict, log, from_seq: int = 0):
        """`agents`: id -> Agent; `first`: id -> its first message (for the roles that start);
        events before `from_seq` (a fork's inherited ones) are not delivered again."""
        self.design, self.branch, self.agents, self.first = design, branch, agents, first
        self.from_seq = from_seq
        self.mail = {a: [] for a in agents}
        self.reported = {a: 0 for a in agents}  # turns whose end was forwarded
        self.wakes = {a: 0 for a in agents}
        self.lock = threading.Lock()
        self.log = log
        self.outcome = None

    def recipients(self, sender: str, to: str | None, text: str) -> list:
        others = [a for a in self.agents if a != sender]
        if to and to != "*":
            return [to] if to in self.agents else []
        topic = re.match(r"\s*#([\w.]+)", text or "")
        if self.design.topics and topic:
            return [a for a in self.design.topics.get(topic.group(1), []) if a != sender]
        return others

    def on_event(self, e: dict) -> None:
        if e.get("kind") not in ("message", "handoff") or e.get("seq", 0) < self.from_seq:
            return
        p, sender = e.get("payload") or {}, e.get("agent")
        text = p.get("text") or p.get("note") or ""
        for to in self.recipients(sender, p.get("to"), text):
            with self.lock:
                self.mail[to].append(f"[{e['kind']} from {sender}, Arena step {e['seq']}]\n{text}")
        self.log({"event": e["kind"], "seq": e["seq"], "from": sender, "to": p.get("to")})

    def forward(self) -> bool:
        """Sends each finished turn's answer to the design's `report_to` role, as the agent's own
        Arena message; whether any was sent."""
        boss, sent = self.design.report_to, False
        for a, agent in self.agents.items():
            if boss is None or a == boss or agent.completed <= self.reported[a]:
                continue
            self.reported[a] = agent.completed
            if agent.last_text.strip():
                arena_api.mcp_call(agent.mcp_url, "message",
                                   {"to": boss, "text": f"[report at the end of my turn]\n{agent.last_text}"})
                sent = True
        return sent

    def deliver(self) -> bool:
        """Gives each idle agent its waiting mail as a turn; whether any agent got some."""
        sent = self.forward()
        for a, agent in self.agents.items():
            with self.lock:
                waiting, self.mail[a] = (self.mail[a], []) if not agent.busy and agent.error is None else ([], self.mail[a])
            if waiting:
                agent.send("\n\n".join(waiting))
                self.log({"turn": a, "messages": len(waiting)})
                sent = True
        return sent

    def idle(self) -> bool:
        with self.lock:
            no_mail = not any(self.mail.values())
        return no_mail and not any(a.busy for a in self.agents.values())

    def statuses(self) -> dict:
        return {a: ("FAILED" if x.error else status_of(x.last_text) if x.turns else "IDLE")
                for a, x in self.agents.items()}

    def done(self) -> bool:
        s = self.statuses()
        if self.design.done_when != "all":
            return s[self.design.done_when] in ("DONE", "FAILED")
        return all(v in ("DONE", "FAILED", "IDLE") for v in s.values())

    def wake(self) -> bool:
        """Wakes each agent still waiting; False when none can be (all out of wakes)."""
        woke = False
        for a, st in self.statuses().items():
            if st == "WAITING" and self.wakes[a] < MAX_WAKES:
                self.wakes[a] += 1
                self.agents[a].send("[wake] Time has passed; the plant's clock keeps running. Check what "
                                    "you are waiting on and carry on.")
                self.log({"wake": a, "n": self.wakes[a]})
                woke = True
        return woke

    def settled(self) -> bool:
        """True when the team stays idle with no mail for QUIET_S."""
        until = time.monotonic() + QUIET_S
        while time.monotonic() < until:
            if self.deliver() or not self.idle():
                return False
            time.sleep(0.5)
        return True

    def usage_limited(self) -> bool:
        """True when Claude Code refused for the subscription's usage limit: the run then says
        nothing about the design, and is stopped."""
        return any("hit your" in (a.error or "") and "limit" in (a.error or "")
                   for a in self.agents.values())

    def run(self, max_s: float) -> str:
        for a, text in self.first.items():
            self.agents[a].send(text)
        deadline = time.monotonic() + max_s
        idle_since = None
        while time.monotonic() < deadline:
            time.sleep(1)
            if self.usage_limited():
                return "usage limit"
            if self.deliver() or not self.idle():
                idle_since = None
                continue
            if not self.settled():
                continue
            if self.done():
                return "done"
            idle_since = idle_since or time.monotonic()
            if time.monotonic() - idle_since >= WAKE_AFTER_S:
                idle_since = None
                if not self.wake():
                    return "stalled"
        return "out of time"
