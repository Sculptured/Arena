"""Typed views of arenad's API objects.

Plain dataclasses. Each object built from an answer keeps the answer itself in ``raw``, so a field
the SDK doesn't model yet (newer cores add fields) is still at hand.
"""

from __future__ import annotations

import datetime as _dt
from dataclasses import dataclass, field
from typing import Any, Literal, Optional

Kind = Literal["claude_code", "codex", "open", "external"]
Runner = Literal["chain", "concurrent", "external"]
Restore = Literal["exact", "approximate"]

#: Branch statuses that mean the branch has ended.
ENDED = frozenset({"passed", "failed", "infra_error", "interrupted", "stopped"})


def _ts(v: Any) -> Optional[_dt.datetime]:
    if not v:
        return None
    return _dt.datetime.fromisoformat(str(v).replace("Z", "+00:00"))


# ---------------------------------------------------------------- teams


@dataclass
class Agent:
    """One agent of a team.

    ``kind`` picks who runs it: ``claude_code`` or ``codex`` (arenad starts the CLI headless, with only
    the world's tools), ``open`` (arenad's own loop, model calls through its gateway to ``provider``),
    or ``external`` (your code drives it through its MCP endpoint and gateway).
    """

    id: str
    kind: Kind = "claude_code"
    model: Optional[str] = None
    prompt: str = ""
    name: Optional[str] = None
    provider: Optional[str] = None
    read_only: bool = False

    @classmethod
    def claude_code(cls, id: str, model: str = "sonnet", prompt: str = "", **kw: Any) -> "Agent":
        return cls(id=id, kind="claude_code", model=model, prompt=prompt, **kw)

    @classmethod
    def codex(cls, id: str, model: Optional[str] = None, prompt: str = "", **kw: Any) -> "Agent":
        return cls(id=id, kind="codex", model=model, prompt=prompt, **kw)

    @classmethod
    def api(cls, id: str, provider: str, model: str, prompt: str = "", **kw: Any) -> "Agent":
        """An API-mode agent: ``provider`` is ``openrouter``, ``openai``, ``anthropic`` or
        ``local:<name>``."""
        return cls(id=id, kind="open", provider=provider, model=model, prompt=prompt, **kw)

    @classmethod
    def external(cls, id: str, model: Optional[str] = None, **kw: Any) -> "Agent":
        return cls(id=id, kind="external", model=model, **kw)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {
            "id": self.id,
            "name": self.name or self.id,
            "kind": self.kind,
            "model": self.model,
            "prompt": self.prompt,
            "read_only": self.read_only,
        }
        if self.provider:
            d["provider"] = self.provider
        return d

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Agent":
        return cls(
            id=d["id"],
            kind=d.get("kind", "claude_code"),
            model=d.get("model"),
            prompt=d.get("prompt") or "",
            name=d.get("name"),
            provider=d.get("provider"),
            read_only=bool(d.get("read_only", False)),
        )


@dataclass
class Team:
    agents: list[Agent] = field(default_factory=list)
    id: str = "team"
    name: str = "Team"

    def to_dict(self) -> dict[str, Any]:
        return {"id": self.id, "name": self.name, "agents": [a.to_dict() for a in self.agents]}

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Team":
        return cls(
            id=d.get("id", "team"),
            name=d.get("name", "Team"),
            agents=[Agent.from_dict(a) for a in d.get("agents", [])],
        )

    def with_all(self, **changes: Any) -> "Team":
        """A copy with every agent changed, e.g. ``team.with_all(model="haiku")``."""
        import dataclasses

        return Team(
            id=self.id,
            name=self.name,
            agents=[dataclasses.replace(a, **changes) for a in self.agents],
        )


def as_team(team: "Team | list[Agent] | dict[str, Any] | None") -> dict[str, Any]:
    if team is None:
        return Team().to_dict()
    if isinstance(team, Team):
        return team.to_dict()
    if isinstance(team, list):
        return Team(agents=list(team)).to_dict()
    return dict(team)


@dataclass
class Change:
    """A change to one agent in a fork. Unset fields are left as they are; ``model=""`` clears the
    model."""

    prompt: Optional[str] = None
    model: Optional[str] = None
    kind: Optional[Kind] = None
    provider: Optional[str] = None
    read_only: Optional[bool] = None

    def to_dict(self) -> dict[str, Any]:
        return {k: v for k, v in self.__dict__.items() if v is not None}


# ---------------------------------------------------------------- scenarios


@dataclass
class Disruption:
    """One of the benchmark's ``scenario_options.disruptions``, chosen with its params (params left
    out take their defaults)."""

    id: str
    params: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        d: dict[str, Any] = {"id": self.id}
        if self.params:
            d["params"] = self.params
        return d


@dataclass
class Scenario:
    disruptions: list[Disruption] = field(default_factory=list)

    def to_dict(self) -> dict[str, Any]:
        return {"disruptions": [d.to_dict() for d in self.disruptions]}


def as_scenario(s: "Scenario | list[Disruption | str] | dict[str, Any] | None") -> Optional[dict]:
    if s is None:
        return None
    if isinstance(s, Scenario):
        return s.to_dict()
    if isinstance(s, list):
        return {
            "disruptions": [
                (Disruption(d) if isinstance(d, str) else d).to_dict() for d in s
            ]
        }
    return dict(s)


# ---------------------------------------------------------------- the benchmark


@dataclass
class Task:
    id: str
    reason: str = ""
    ticket: str = ""
    instructions: str = ""
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Task":
        return cls(
            id=str(d["id"]),
            reason=d.get("reason", ""),
            ticket=d.get("ticket", ""),
            instructions=d.get("instructions", ""),
            raw=d,
        )


@dataclass
class Benchmark:
    name: str
    version: str
    tasks: list[Task]
    #: The disruptions a run's scenario may add (``{disruptions: [...]}``), or None.
    scenario_options: Optional[dict[str, Any]]
    #: True when the benchmark draws the world's view (the digital twin).
    view: bool
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Benchmark":
        return cls(
            name=d["name"],
            version=d.get("version", ""),
            tasks=[Task.from_dict(t) for t in d.get("tasks", [])],
            scenario_options=d.get("scenario_options"),
            view=bool(d.get("view", False)),
            raw=d,
        )

    @property
    def disruptions(self) -> list[dict[str, Any]]:
        return list((self.scenario_options or {}).get("disruptions", []))


# ---------------------------------------------------------------- records


@dataclass
class Event:
    """One recorded step of a branch. ``kind`` is ``run_start``, ``agent_start``, ``model_call``,
    ``tool_call``, ``handoff``, ``message``, ``agent_end``, ``check``, ``world_event``,
    ``world_view`` (and on newer cores ``pause``/``resume``); ``payload`` its kind's fields (see
    docs/dataset.md)."""

    branch_id: str
    seq: int
    at: Optional[_dt.datetime]
    agent: Optional[str]
    kind: str
    payload: dict[str, Any]

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Event":
        return cls(
            branch_id=d.get("branch_id", ""),
            seq=int(d["seq"]),
            at=_ts(d.get("at")),
            agent=d.get("agent"),
            kind=d["kind"],
            payload=d.get("payload") or {},
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            "branch_id": self.branch_id,
            "seq": self.seq,
            "at": self.at.isoformat() if self.at else None,
            "agent": self.agent,
            "kind": self.kind,
            "payload": self.payload,
        }

    # Shortcuts for tool calls.
    @property
    def tool(self) -> Optional[str]:
        return self.payload.get("name") if self.kind == "tool_call" else None

    @property
    def error(self) -> Optional[str]:
        return self.payload.get("error")

    @property
    def writes(self) -> list[str]:
        return list(self.payload.get("writes") or [])

    def summary(self) -> str:
        """One line for a console timeline."""
        who = self.agent or "arena"
        p = self.payload
        if self.kind == "tool_call":
            out = f"error: {p['error']}" if p.get("error") else "ok"
            w = f" writes {','.join(p['writes'])}" if p.get("writes") else ""
            return f"{self.seq:>4} {who:<12} {p.get('name')}({_short(p.get('args'))}) -> {out}{w}"
        if self.kind in ("handoff", "message"):
            to = p.get("to") or "next"
            return f"{self.seq:>4} {who:<12} {self.kind} to {to}: {_short(p.get('note') or p.get('text'), 100)}"
        if self.kind == "check":
            return f"{self.seq:>4} {who:<12} check: {'passed' if p.get('passed') else 'failed'}"
        if self.kind == "agent_end":
            return f"{self.seq:>4} {who:<12} end ({'ok' if p.get('ok') else p.get('error')})"
        if self.kind == "world_event":
            return f"{self.seq:>4} {p.get('part', 'world'):<12} {p.get('kind')}: {_short(p.get('text'), 100)}"
        return f"{self.seq:>4} {who:<12} {self.kind}"


def _short(v: Any, n: int = 60) -> str:
    import json

    s = v if isinstance(v, str) else json.dumps(v, separators=(",", ":"), default=str)
    s = " ".join(str(s).split())
    return s if len(s) <= n else s[: n - 1] + "…"


@dataclass
class NamedCheck:
    name: str
    passed: bool
    detail: str = ""
    safety: bool = False


@dataclass
class CheckResult:
    """How a branch was scored (its ``check`` event)."""

    passed: bool
    #: The benchmark's named checks; for a benchmark scored against its gold world, one
    #: ``matches_gold`` check.
    checks: list[NamedCheck]
    differing_rows: list[str]
    unchecked: list[str]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_payload(cls, p: dict[str, Any]) -> "CheckResult":
        named = p.get("checks")
        if named:
            checks = [
                NamedCheck(
                    c["name"], bool(c["passed"]), c.get("detail", ""), bool(c.get("safety", False))
                )
                for c in named
            ]
        else:
            checks = [NamedCheck("matches_gold", bool(p.get("passed")))]
        return cls(
            passed=bool(p.get("passed")),
            checks=checks,
            differing_rows=list(p.get("differing_rows") or []),
            unchecked=list(p.get("unchecked") or []),
            raw=p,
        )

    @property
    def safety_violations(self) -> list[NamedCheck]:
        return [c for c in self.checks if c.safety and not c.passed]

    @property
    def score(self) -> float:
        """Share of goal checks (safety checks left out) passed."""
        goals = [c for c in self.checks if not c.safety]
        return sum(c.passed for c in goals) / len(goals) if goals else float(self.passed)


@dataclass
class BranchRecord:
    """A branch as ``GET /api/branches/{id}`` gives it: its team, status, events and metrics."""

    id: str
    run_id: str
    status: str
    error: Optional[str]
    parent_id: Optional[str]
    start_seq: Optional[int]
    team: Team
    events: list[Event]
    metrics: dict[str, Any]
    created_at: Optional[_dt.datetime]
    ended_at: Optional[_dt.datetime]
    restore_mode: Optional[str]
    restored: list[dict[str, Any]]
    fork_note: Optional[str]
    #: True while paused (None on a core without pause).
    paused: Optional[bool]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "BranchRecord":
        return cls(
            id=d["id"],
            run_id=d["run_id"],
            status=d["status"],
            error=d.get("error"),
            parent_id=d.get("parent_id"),
            start_seq=d.get("start_seq"),
            team=Team.from_dict(d.get("team") or {}),
            events=[Event.from_dict(e) for e in d.get("events", [])],
            metrics=d.get("metrics") or {},
            created_at=_ts(d.get("created_at")),
            ended_at=_ts(d.get("ended_at")),
            restore_mode=d.get("restore_mode"),
            restored=list(d.get("restored") or []),
            fork_note=d.get("fork_note"),
            paused=d.get("paused"),
            raw=d,
        )

    @property
    def ended(self) -> bool:
        return self.status in ENDED

    @property
    def run_start(self) -> dict[str, Any]:
        """The ``RunStart`` payload: task, team, benchmark, world, seed, runner, scenario, tools."""
        for e in self.events:
            if e.kind == "run_start":
                return e.payload
        return {}

    @property
    def seed(self) -> Optional[int]:
        return self.run_start.get("seed")

    @property
    def check(self) -> Optional[CheckResult]:
        """The branch's score, once it has one."""
        for e in reversed(self.events):
            if e.kind == "check":
                return CheckResult.from_payload(e.payload)
        return None

    def tool_calls(self, agent: Optional[str] = None) -> list[Event]:
        return [
            e for e in self.events if e.kind == "tool_call" and (agent is None or e.agent == agent)
        ]


@dataclass
class RunSummary:
    id: str
    task_id: str
    name: Optional[str]
    created_at: Optional[_dt.datetime]
    branches: list[dict[str, Any]]
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "RunSummary":
        return cls(
            id=d["id"],
            task_id=str(d["task_id"]),
            name=d.get("name"),
            created_at=_ts(d.get("created_at")),
            branches=list(d.get("branches", [])),
            raw=d,
        )


@dataclass
class ForkInfo:
    """Where a fork started and why it moved (``moved_from``/``reason`` when it did)."""

    start_seq: int
    moved_from: Optional[int]
    reason: Optional[str]
