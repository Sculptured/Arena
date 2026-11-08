"""The client: :class:`Arena` (a connection to one arenad), :class:`Branch`, :class:`AgentEndpoint` and
:class:`Suite`."""

from __future__ import annotations

import time
from typing import Any, AsyncIterator, Iterator, Optional, Sequence, Union

import httpx

from . import sse
from .errors import ApiError, ConnectionFailed, NotFound, NotSupported, from_status
from .mcp import McpClient, Tool, ToolResult
from .models import (
    ENDED,
    Agent,
    Benchmark,
    BranchRecord,
    Change,
    CheckResult,
    Event,
    ForkInfo,
    RunSummary,
    Task,
    Team,
    as_scenario,
    as_team,
)

DEFAULT_URL = "http://127.0.0.1:4510"

TeamLike = Union[Team, Sequence[Agent], dict, None]


class Arena:
    """A connection to a running arenad.

    >>> arena = Arena("http://127.0.0.1:4510")
    >>> arena.benchmark().name
    'retail'

    Use :meth:`Arena.local` to start an arenad of your own for the length of a ``with`` block.
    """

    def __init__(self, url: str = DEFAULT_URL, timeout: float = 900.0, retries: int = 3):
        self.url = url.rstrip("/")
        self.retries = retries
        self._http = httpx.Client(
            base_url=self.url, timeout=httpx.Timeout(timeout, connect=10.0)
        )
        self._pause_supported: Optional[bool] = None

    # -------------------------------------------------------------- plumbing

    def _request(self, method: str, path: str, *, json: Any = None, params: Any = None) -> Any:
        """One API call; idempotent reads are tried again when arenad can't be reached (it may be
        restarting)."""
        tries = self.retries if method == "GET" else 1
        for attempt in range(tries):
            try:
                r = self._http.request(method, path, json=json, params=params)
                break
            except httpx.TransportError as e:
                if attempt + 1 >= tries:
                    raise ConnectionFailed(f"arenad at {self.url}: {e}") from e
                time.sleep(min(2**attempt, 8))
        if r.status_code >= 400:
            try:
                msg = r.json().get("error", r.text)
            except ValueError:
                msg = r.text
            raise from_status(r.status_code, msg, method, path)
        if not r.content:
            return None
        return r.json()

    def get(self, path: str, **params: Any) -> Any:
        """``GET`` any API route (for routes the SDK doesn't wrap yet)."""
        return self._request("GET", path, params=params or None)

    def post(self, path: str, body: Any = None) -> Any:
        """``POST`` any API route with a JSON body."""
        return self._request("POST", path, json=body if body is not None else {})

    def close(self) -> None:
        self._http.close()

    def __enter__(self) -> "Arena":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def __repr__(self) -> str:
        return f"Arena({self.url!r})"

    # -------------------------------------------------------------- starting a core

    @classmethod
    def local(cls, **kw: Any) -> "Arena":
        """Starts arenad as a subprocess and connects to it; it stops when the ``with`` block (or
        :meth:`close`) ends. See :func:`arena.local.start` for the arguments."""
        from .local import LocalArena

        return LocalArena(**kw)

    # -------------------------------------------------------------- what the core offers

    def health(self) -> dict[str, Any]:
        """Benchmark, CLIs found, model providers, disk use."""
        return self.get("/api/health")

    def providers(self) -> list[dict[str, Any]]:
        """Each model provider with whether it is available and (when asked) its models."""
        return self.get("/api/providers")

    def benchmark(self) -> Benchmark:
        return Benchmark.from_dict(self.get("/api/benchmark"))

    def tasks(self) -> list[Task]:
        """The benchmark's tasks with their ticket and the agents' instructions."""
        return [Task.from_dict(t) for t in self.get("/api/tasks")]

    def task(self, task_id: str) -> Task:
        for t in self.tasks():
            if t.id == str(task_id):
                return t
        raise NotFound(404, f"the benchmark has no task {task_id!r}")

    def default_team(
        self, kind: Optional[str] = None, model: Optional[str] = None, provider: Optional[str] = None
    ) -> Team:
        """The team the benchmark suggests; ``kind``/``model``/``provider`` set every agent's."""
        q = {k: v for k, v in {"kind": kind, "model": model, "provider": provider}.items() if v}
        return Team.from_dict(self.get("/api/team/default", **q))

    def world(self) -> list[dict[str, Any]]:
        """The world's parts and what each integration reports (restore, clock, tools, system)."""
        return self.get("/api/world")

    def usage(self) -> dict[str, Any]:
        return self.get("/api/usage")

    def gc(self) -> dict[str, Any]:
        return self.post("/api/gc")

    # -------------------------------------------------------------- runs

    def run(
        self,
        task: Union[str, Task],
        team: TeamLike = None,
        runner: str = "chain",
        scenario: Any = None,
        seed: Optional[int] = None,
        name: Optional[str] = None,
    ) -> "Branch":
        """Starts a run of ``task`` and returns its first branch.

        ``runner`` is ``chain`` (arenad runs the agents one after another), ``concurrent`` (all at
        once) or ``external`` (your code drives them; prefer :meth:`external_run`). ``team`` defaults
        to the benchmark's suggested team. ``scenario`` is a :class:`Scenario`, a list of disruption
        ids / :class:`Disruption`, or the raw dict. ``seed`` fixes the world's seed (random when
        absent; recorded either way).
        """
        task_id = task.id if isinstance(task, Task) else str(task)
        if team is None and runner != "external":
            team = self.default_team()
        body: dict[str, Any] = {"task_id": task_id, "team": as_team(team), "runner": runner}
        sc = as_scenario(scenario)
        if sc:
            body["scenario"] = sc
        if seed is not None:
            body["seed"] = int(seed)
        ans = self.post("/api/runs", body)
        branch = Branch(self, ans["branch_id"], ans["run_id"], task_id=task_id)
        for ep in ans.get("agents") or []:
            branch.agents[ep["agent"]] = AgentEndpoint.from_answer(branch, ep)
        if name:
            self.post(f"/api/runs/{ans['run_id']}/name", {"name": name})
        return branch

    def external_run(
        self,
        task: Union[str, Task],
        agents: Sequence[Union[Agent, str]] = (),
        scenario: Any = None,
        seed: Optional[int] = None,
        name: Optional[str] = None,
    ) -> "Branch":
        """Starts a run your code drives and registers ``agents`` (ids or :class:`Agent`, kind
        ``external`` by default). Each gets an MCP endpoint (the world's tools) and a gateway (an
        OpenAI-compatible base URL): ``branch.agents[id].mcp_url`` / ``.gateway_url``. Call
        :meth:`Branch.finish` when the team is done; arenad then scores the world."""
        branch = self.run(task, team=[], runner="external", scenario=scenario, seed=seed, name=name)
        for a in agents:
            branch.register(a)
        return branch

    def runs(self) -> list[RunSummary]:
        return [RunSummary.from_dict(r) for r in self.get("/api/runs")]

    def branch(self, branch_id: str) -> "Branch":
        rec = self.get(f"/api/branches/{branch_id}")
        return Branch(self, rec["id"], rec["run_id"], task_id=None, record=rec)

    def delete_run(self, run_id: str) -> None:
        """Deletes a run and what it saved (refused while a branch of it runs)."""
        self._request("DELETE", f"/api/runs/{run_id}")

    def name_run(self, run_id: str, name: str) -> None:
        self.post(f"/api/runs/{run_id}/name", {"name": name})

    def compare(self, a: Union[str, "Branch"], b: Union[str, "Branch"]) -> dict[str, Any]:
        """Two ended branches: ``first_divergent_seq``, ``differing_rows``, metrics side by side."""
        return self.get("/api/compare", a=_bid(a), b=_bid(b))

    # -------------------------------------------------------------- suites

    def suite(
        self,
        team: TeamLike = None,
        tasks: Optional[Sequence[str]] = None,
        repeats: int = 1,
        runner: str = "chain",
        name: Optional[str] = None,
        scenario: Any = None,
        scenarios: Optional[Sequence[Any]] = None,
    ) -> "Suite":
        """A test suite: ``team`` over ``tasks`` (all when None), each ``repeats`` times, on the same
        seeded world and checks. With ``runner="external"`` your code claims and drives each run
        (:meth:`Suite.claim`)."""
        if team is None and runner != "external":
            team = self.default_team()
        body: dict[str, Any] = {"team": as_team(team), "repeats": repeats, "runner": runner}
        if tasks is not None:
            body["tasks"] = [str(t) for t in tasks]
        if name:
            body["name"] = name
        if scenario is not None:
            body["scenario"] = as_scenario(scenario)
        if scenarios is not None:
            body["scenarios"] = [as_scenario(s) for s in scenarios]
        return Suite(self, self.post("/api/suites", body))

    def suites(self) -> list[dict[str, Any]]:
        return self.get("/api/suites")

    def get_suite(self, suite_id: str) -> "Suite":
        return Suite(self, self.get(f"/api/suites/{suite_id}"))

    def compare_suites(self, a: Union[str, "Suite"], b: Union[str, "Suite"]) -> dict[str, Any]:
        """Suite ``b`` against ``a``: each check's pass rate, safety, time, tokens and cost, with a
        verdict (``improved``, ``regressed``, ``same``, ``incomparable``)."""
        ida = a.id if isinstance(a, Suite) else a
        idb = b.id if isinstance(b, Suite) else b
        return self.get("/api/suites/compare", a=ida, b=idb)

    # -------------------------------------------------------------- AI review

    def review_settings(self) -> dict[str, Any]:
        """``{enabled, model, why?}``: whether arenad's AI review is on, and with which model."""
        return self.get("/api/review/settings")

    def _review(
        self, path: str, params: dict[str, Any], again: bool, wait: bool, timeout: float, poll: float
    ) -> dict[str, Any]:
        """Starts (or reads the stored) review at ``path``; with ``wait``, until it has ended."""
        asked = {**params, "again": "1"} if again else params
        first = self._request("POST", path, params=asked)
        if not wait:
            return first
        return wait_review(lambda: self.get(path, **params), first, timeout, poll)

    def compare_review(
        self,
        a: Union[str, "Branch"],
        b: Union[str, "Branch"],
        again: bool = False,
        wait: bool = True,
        timeout: float = 900.0,
        poll: float = 3.0,
    ) -> dict[str, Any]:
        """An AI review of two ended branches: ``review`` is ``{headline, differences:
        [{title, a, b, why_it_matters, steps_a, steps_b}], which_is_better, why}``. Stored with the
        pair; ``again=True`` asks for a new one. Runs a real model (it costs what the model costs)."""
        return self._review("/api/compare/review", {"a": _bid(a), "b": _bid(b)}, again, wait, timeout, poll)

    def compare_suites_review(
        self,
        a: Union[str, "Suite"],
        b: Union[str, "Suite"],
        again: bool = False,
        wait: bool = True,
        timeout: float = 900.0,
        poll: float = 3.0,
    ) -> dict[str, Any]:
        """An AI review of suite ``b`` against ``a``, citing example runs (branch ids) in
        ``steps_a``/``steps_b``. As :meth:`compare_review`."""
        ida = a.id if isinstance(a, Suite) else a
        idb = b.id if isinstance(b, Suite) else b
        params = {"a": ida, "b": idb}
        return self._review("/api/suites/compare/review", params, again, wait, timeout, poll)

    # -------------------------------------------------------------- features

    def supports_pause(self) -> bool:
        """True when this arenad has pause, resume and stop (``POST /api/branches/{id}/pause``)."""
        if self._pause_supported is None:
            try:
                self.post("/api/branches/arena-sdk-probe/pause")
                self._pause_supported = True
            except NotFound as e:
                self._pause_supported = "no such route" not in e.message
            except ApiError:
                self._pause_supported = True
        return self._pause_supported


def wait_review(
    fetch: Any, first: dict[str, Any], timeout: float, poll: float, sleep: Any = time.sleep
) -> dict[str, Any]:
    """``first``, or what ``fetch()`` gives, once its ``status`` is no longer ``running``
    (``done`` with ``review``, or ``error`` with ``error``); TimeoutError after ``timeout`` s."""
    deadline = time.monotonic() + timeout
    r = first
    while r.get("status") == "running":
        if time.monotonic() > deadline:
            raise TimeoutError(f"the review {r.get('key')} is still running after {timeout} s")
        sleep(poll)
        r = fetch()
    return r


def _bid(b: Union[str, "Branch"]) -> str:
    return b.id if isinstance(b, Branch) else b


class Branch:
    """A branch of a run: the original run, or a fork. Cheap to make; reads go to arenad each time."""

    def __init__(
        self,
        arena: Arena,
        branch_id: str,
        run_id: str,
        task_id: Optional[str] = None,
        record: Optional[dict[str, Any]] = None,
        fork: Optional[ForkInfo] = None,
    ):
        self.arena = arena
        self.id = branch_id
        self.run_id = run_id
        self.task_id = task_id
        #: The endpoints of the agents your code drives on this branch (external runs).
        self.agents: dict[str, AgentEndpoint] = {}
        #: Where a fork started (None for a run's first branch).
        self.fork_info = fork
        #: The final ``{status, error}`` once :meth:`stream` saw the end.
        self.final: Optional[dict[str, Any]] = None
        if record is not None and self.task_id is None:
            self.task_id = _task_of(record)

    def __repr__(self) -> str:
        return f"Branch({self.id!r}, run={self.run_id!r})"

    # -------------------------------------------------------------- reading

    def record(self) -> BranchRecord:
        """The branch now: status, team, every event, metrics."""
        rec = BranchRecord.from_dict(self.arena.get(f"/api/branches/{self.id}"))
        if self.task_id is None:
            self.task_id = rec.run_start.get("task_id")
        return rec

    refresh = record

    @property
    def status(self) -> str:
        return self.record().status

    def events(self) -> list[Event]:
        return self.record().events

    def metrics(self) -> dict[str, Any]:
        """Measured and estimated time, overhead per kind, tokens and cost per agent."""
        return self.record().metrics

    def check(self) -> Optional[CheckResult]:
        """The branch's score (None until it has ended and been scored)."""
        return self.record().check

    def views(self) -> list[dict[str, Any]]:
        """The world's views (the digital twin) after each step: ``[{seq, event_seq, view}]``."""
        return self.arena.get(f"/api/branches/{self.id}/views")

    def rows(self, seq: int, keys: Sequence[str], part: Optional[str] = None) -> dict[str, Any]:
        """The state of ``keys`` (``<kind>/<id>``, at most 50) just after step ``seq``."""
        q: dict[str, Any] = {"seq": seq, "keys": ",".join(keys)}
        if part:
            q["part"] = part
        return self.arena.get(f"/api/branches/{self.id}/rows", **q)

    def stream(self, reconnect: int = 5) -> Iterator[Event]:
        """The branch's events as they happen: first those recorded so far, then each new one,
        ending when the branch ends (``self.final`` then holds ``{status, error}``). If the
        connection drops (arenad restarting), it reconnects up to ``reconnect`` times and goes on
        after the last event it gave."""
        seen = [0]  # the last seq given
        tries = 0
        while True:
            before = seen[0]
            try:
                yield from self._stream_once(seen)
                if self.final is not None:
                    return
            except httpx.TransportError:
                pass
            tries = 1 if seen[0] > before else tries + 1
            if tries > reconnect:
                raise ConnectionFailed(f"lost the event stream of branch {self.id}")
            time.sleep(min(2**tries, 10))
            # The branch may have ended while the stream was down.
            if (yield from self._ended_meanwhile(seen)):
                return

    def _stream_once(self, seen: list[int]) -> Iterator[Event]:
        """One connection to the event stream: the events after ``seen[0]``; sets ``final`` at the
        end."""
        self.final = None
        path = f"/api/branches/{self.id}/stream"
        with self.arena._http.stream("GET", path, timeout=httpx.Timeout(None, connect=10.0)) as r:
            if r.status_code >= 400:
                r.read()
                raise from_status(r.status_code, r.text, "GET", path)
            for name, data in sse.parse(r.iter_lines()):
                if name == "status":
                    self.final = data
                    return
                if name == "event" and data["seq"] > seen[0]:
                    seen[0] = data["seq"]
                    yield Event.from_dict(data)

    def _ended_meanwhile(self, seen: list[int]):
        """After a dropped stream: if the branch has ended, yields the events not given yet, sets
        ``final`` and returns True."""
        try:
            rec = self.record()
        except ConnectionFailed:
            return False
        if not rec.ended:
            return False
        for e in rec.events:
            if e.seq > seen[0]:
                seen[0] = e.seq
                yield e
        self.final = {"status": rec.status, "error": rec.error}
        return True

    async def astream(self) -> AsyncIterator[Event]:
        """:meth:`stream` for asyncio (no reconnect)."""
        async with httpx.AsyncClient(
            base_url=self.arena.url, timeout=httpx.Timeout(None, connect=10.0)
        ) as client:
            async with client.stream("GET", f"/api/branches/{self.id}/stream") as r:
                if r.status_code >= 400:
                    await r.aread()
                    raise from_status(r.status_code, r.text, "GET", "stream")
                async for name, data in sse.aparse(r.aiter_lines()):
                    if name == "event":
                        yield Event.from_dict(data)
                    elif name == "status":
                        self.final = data
                        return

    def wait(self, timeout: Optional[float] = None, poll: float = 2.0) -> BranchRecord:
        """Waits for the branch to end and returns its record."""
        deadline = None if timeout is None else time.monotonic() + timeout
        while True:
            rec = self.record()
            if rec.ended:
                return rec
            if deadline is not None and time.monotonic() > deadline:
                raise TimeoutError(f"branch {self.id} still {rec.status} after {timeout} s")
            time.sleep(poll)

    def review(
        self, again: bool = False, wait: bool = True, timeout: float = 900.0, poll: float = 3.0
    ) -> dict[str, Any]:
        """An AI review of this ended branch: where it went wrong and what to change. ``review`` is
        ``{verdict, went_wrong: [{title, why, steps, severity}], suggestions: [{change, why, kind,
        agent?, prompt_text?, step?}], went_well, confidence}``, with ``model``, ``cost_usd`` and
        ``issues`` (citations arenad dropped). Stored with the branch; ``again=True`` asks for a new
        one. Runs a real model (it costs what the model costs); 409 when AI review is off."""
        path = f"/api/branches/{self.id}/review"
        return self.arena._review(path, {}, again, wait, timeout, poll)

    # -------------------------------------------------------------- forks

    def restore_plan(self, at_seq: int, restore: str = "exact") -> dict[str, Any]:
        """How each part would be restored for a fork at ``at_seq`` (409 with the refusal)."""
        return self.arena.get(
            f"/api/branches/{self.id}/restore-plan", at_seq=at_seq, restore=restore
        )

    def fork(
        self,
        at_seq: int,
        changes: Optional[dict[str, Union[Change, dict[str, Any]]]] = None,
        restore: str = "exact",
        team: TeamLike = None,
    ) -> "Branch":
        """A new branch from just before step ``at_seq``, with ``changes`` to agents
        (``{agent id: Change(...)}``) or, for an external run, another ``team``. The world is
        restored ``exact`` or ``approximate``; a part that can't be is refused (:class:`Conflict`).
        The fork may start earlier than asked (``fork_info.moved_from``/``reason``). A fork of an
        external run stays live: its ``agents`` hold each agent's endpoints and ``history``."""
        body: dict[str, Any] = {"at_seq": at_seq, "restore": restore}
        if changes:
            body["changes"] = {
                k: (v.to_dict() if isinstance(v, Change) else dict(v)) for k, v in changes.items()
            }
        if team is not None:
            body["team"] = as_team(team)
        ans = self.arena.post(f"/api/branches/{self.id}/fork", body)
        info = ForkInfo(ans["start_seq"], ans.get("moved_from"), ans.get("reason"))
        b = Branch(self.arena, ans["branch_id"], self.run_id, task_id=self.task_id, fork=info)
        for ep in ans.get("agents") or []:
            b.agents[ep["agent"]] = AgentEndpoint.from_answer(b, ep)
        return b

    # -------------------------------------------------------------- external runs

    def register(self, agent: Union[Agent, str], **kw: Any) -> "AgentEndpoint":
        """Registers an agent on this live external branch (or starts a known one again) and
        returns its endpoints."""
        if isinstance(agent, str):
            agent = Agent(id=agent, kind=kw.pop("kind", "external"), **kw)
        body = {k: v for k, v in agent.to_dict().items() if v is not None}
        ans = self.arena.post(f"/api/branches/{self.id}/agents", body)
        ep = AgentEndpoint.from_answer(self, ans)
        self.agents[ep.id] = ep
        return ep

    def agent(self, agent_id: str) -> "AgentEndpoint":
        return self.agents[agent_id]

    def finish(self) -> dict[str, Any]:
        """External runs: freeze, score and stop the world. Returns ``{status, error}``."""
        ans = self.arena.post(f"/api/branches/{self.id}/finish")
        self.final = ans
        return ans

    # -------------------------------------------------------------- control

    def _flow(self, what: str) -> dict[str, Any]:
        try:
            return self.arena.post(f"/api/branches/{self.id}/{what}")
        except NotFound as e:
            if "no such route" in e.message:
                raise NotSupported(
                    f"this arenad has no {what} (POST /api/branches/{{id}}/{what}); "
                    "update arenad to pause, resume or stop a live branch"
                ) from e
            raise

    def pause(self) -> dict[str, Any]:
        """Holds new tool calls and gateway model calls and stops the world clock (newer cores
        only: raises :class:`NotSupported` otherwise; see :meth:`Arena.supports_pause`)."""
        return self._flow("pause")

    def resume(self) -> dict[str, Any]:
        return self._flow("resume")

    def stop(self) -> dict[str, Any]:
        """Ends a live branch ``stopped``, unscored (newer cores only)."""
        return self._flow("stop")

    # -------------------------------------------------------------- export

    def export(self, path: str) -> str:
        """Writes the branch as a dataset folder (``events.jsonl``, ``branches.jsonl``,
        ``SCHEMA.md``); see :mod:`arena.export`."""
        from .export import export_branches

        return export_branches(self.arena, [self.id], path)

    def delete_run(self) -> None:
        self.arena.delete_run(self.run_id)


def _task_of(rec: dict[str, Any]) -> Optional[str]:
    for e in rec.get("events", []):
        if e.get("kind") == "run_start":
            return (e.get("payload") or {}).get("task_id")
    return None


class AgentEndpoint:
    """One agent your code drives: its MCP endpoint (``mcp_url``: the world's tools plus
    ``handoff``/``message``/``inbox``) and its gateway (``gateway_url``: an OpenAI-compatible base
    URL whose calls go to the agent's provider and are recorded)."""

    def __init__(
        self,
        branch: Branch,
        agent_id: str,
        mcp_url: str,
        gateway_url: str,
        tools: Optional[list[Tool]] = None,
        spec: Optional[dict[str, Any]] = None,
        history: Optional[list[Event]] = None,
    ):
        self.branch = branch
        self.id = agent_id
        self.mcp_url = mcp_url
        self.gateway_url = gateway_url
        self.spec = spec or {}
        #: The agent's recorded events before a fork point (forks of external runs).
        self.history = history or []
        self._tools = tools
        self.mcp = McpClient(mcp_url, branch.arena._http)

    @classmethod
    def from_answer(cls, branch: Branch, d: dict[str, Any]) -> "AgentEndpoint":
        tools = [Tool.from_dict(t) for t in d["tools"]] if d.get("tools") is not None else None
        hist = [Event.from_dict(e) for e in d.get("history") or []]
        return cls(branch, d["agent"], d["mcp_url"], d["gateway_url"], tools, d.get("spec"), hist)

    def __repr__(self) -> str:
        return f"AgentEndpoint({self.id!r}, branch={self.branch.id!r})"

    @property
    def tools(self) -> list[Tool]:
        """The tools this agent may call (a read-only agent gets no write tools)."""
        if self._tools is None:
            self._tools = self.mcp.list_tools()
        return self._tools

    def call(self, name: str, arguments: Optional[dict[str, Any]] = None, **kw: Any) -> ToolResult:
        """Calls a tool as this agent; arenad runs it in the world and records it."""
        return self.mcp.call_tool(name, {**(arguments or {}), **kw})

    def report_usage(self, **usage: Any) -> int:
        """Records one model call your agent's client made outside the gateway: ``input_tokens``,
        ``output_tokens``, ``cache_creation_input_tokens``, ``cache_read_input_tokens``, ``model``,
        ``message_id``, ``context_window``. Returns the event's seq."""
        return self.branch.arena.post(
            f"/api/branches/{self.branch.id}/agents/{self.id}/usage", usage
        )["seq"]

    def report_compaction(self, trigger: Optional[str] = None, pre_tokens: Optional[int] = None) -> int:
        c = {k: v for k, v in {"trigger": trigger, "pre_tokens": pre_tokens}.items() if v is not None}
        return self.branch.arena.post(
            f"/api/branches/{self.branch.id}/agents/{self.id}/usage", {"compaction": c}
        )["seq"]

    def end(self, ok: bool = True, error: Optional[str] = None, **totals: Any) -> int:
        """Records that the agent is done; ``totals``: ``prompt_tokens``, ``completion_tokens``,
        ``cost_usd``, ``cost_source``, ``seconds``."""
        body = {"ok": ok, **({"error": error} if error else {}), **totals}
        return self.branch.arena.post(
            f"/api/branches/{self.branch.id}/agents/{self.id}/end", body
        )["seq"]

    def mcp_config(self, name: str = "arena") -> dict[str, Any]:
        """An ``mcpServers`` config for Claude Code (``--mcp-config``) and other MCP hosts."""
        return {"mcpServers": {name: {"type": "http", "url": self.mcp_url}}}


class Suite:
    """A test suite. ``report`` is the last report read; :meth:`refresh` reads it again."""

    def __init__(self, arena: Arena, report: dict[str, Any]):
        self.arena = arena
        self.id: str = report["id"]
        self.report = report

    def __repr__(self) -> str:
        return f"Suite({self.id!r}, status={self.report.get('status')!r})"

    def refresh(self) -> dict[str, Any]:
        self.report = self.arena.get(f"/api/suites/{self.id}")
        return self.report

    @property
    def status(self) -> str:
        return self.refresh()["status"]

    @property
    def summary(self) -> dict[str, Any]:
        return self.report.get("summary") or {}

    def wait(self, timeout: Optional[float] = None, poll: float = 5.0) -> dict[str, Any]:
        """Waits until the suite is no longer ``running`` (arenad runs it) and returns the report."""
        deadline = None if timeout is None else time.monotonic() + timeout
        while self.refresh()["status"] == "running":
            if deadline is not None and time.monotonic() > deadline:
                raise TimeoutError(f"suite {self.id} still running after {timeout} s")
            time.sleep(poll)
        return self.report

    def claim(self) -> Optional[Branch]:
        """External suites: claims the next run and starts it as an external run; None when every
        run is claimed. Register agents and drive it like :meth:`Arena.external_run`'s branch."""
        ans = self.arena.post(f"/api/suites/{self.id}/claim")
        if ans.get("done"):
            return None
        b = Branch(self.arena, ans["branch_id"], ans["run_id"], task_id=str(ans["task_id"]))
        b.suite_slot = {"variant": ans.get("variant"), "repeat": ans.get("repeat")}  # type: ignore[attr-defined]
        for ep in ans.get("agents") or []:
            b.agents[ep["agent"]] = AgentEndpoint.from_answer(b, ep)
        return b

    def compare(self, other: Union[str, "Suite"]) -> dict[str, Any]:
        """``other`` against this suite."""
        return self.arena.compare_suites(self, other)

    def branches(self) -> list[Branch]:
        return [
            Branch(self.arena, r["branch_id"], r["run_id"], task_id=str(r["task_id"]))
            for r in self.refresh()["runs"]
            if r.get("branch_id")
        ]

    def export(self, path: str) -> str:
        """Every run of the suite as one dataset folder (see :mod:`arena.export`)."""
        from .export import export_branches

        return export_branches(self.arena, [b.id for b in self.branches()], path, suite=self.report)


__all__ = ["Arena", "Branch", "AgentEndpoint", "Suite", "ENDED", "DEFAULT_URL"]
