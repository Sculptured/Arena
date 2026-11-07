"""A multi-agent step-loop environment over an Arena world, for RL and agent research.

It follows PettingZoo's parallel API (``reset``/``step``/``close``, dicts keyed by agent) without
depending on PettingZoo. Arena supplies the world, the record and the score; **your policy** (an LLM
or anything else) picks the actions. Arena supplies no agent.

    env = ArenaParallelEnv(arena, task="72", agents=["support"])
    obs, infos = env.reset(seed=7)
    while env.agents:
        actions = {a: my_policy(obs[a]) for a in env.agents}   # {"name": tool, "arguments": {...}}
        obs, rewards, terminations, truncations, infos = env.step(actions)
    env.close()

- An action is a tool call ``{"name": ..., "arguments": {...}}`` made through the agent's MCP endpoint
  (so it is recorded like any agent's), ``None`` to do nothing this step, or :data:`END` when the
  agent is done. Actions of one step run one after another, in the order of ``possible_agents``.
- An observation is a dict: ``tool_result`` (the last call's :class:`~arena.mcp.ToolResult`, as
  ``{"text", "is_error", "value"}``), ``messages`` (notes and messages sent to the agent since its
  last step), ``world_events`` (what the world's parts reported since then), and on reset ``prompt``,
  ``ticket``, ``instructions`` and ``tools`` (the tools the agent may call, as dicts).
- The episode ends when every agent has sent :data:`END` (terminations) or after ``max_steps``
  steps (truncations). The branch is then finished and scored: the terminal reward is 1.0/0.0 for a
  pass/fail (``reward="pass"``) or the share of goal checks passed (``reward="score"``), given to
  every agent. ``shaping(env, agent, action, observation) -> float`` adds a per-step reward.
- :meth:`fork` gives "reset to step N" for counterfactual rollouts: ``env2, obs, infos =
  env.fork(N)`` is a new env on a fork of the finished episode, the world restored as it was just
  before step N.
"""

from __future__ import annotations

import copy
from typing import Any, Callable, Optional, Sequence, Union

from .client import AgentEndpoint, Arena, Branch
from .models import Agent, CheckResult, Event, Task

#: The action that says an agent is done.
END = "__end__"

Shaping = Callable[["ArenaParallelEnv", str, Any, dict], float]


class ArenaParallelEnv:
    metadata = {"name": "arena_parallel_v0", "is_parallelizable": False}

    def __init__(
        self,
        arena: Arena,
        task: Union[str, Task],
        agents: Sequence[Union[str, Agent]],
        scenario: Any = None,
        max_steps: int = 50,
        reward: str = "pass",
        shaping: Optional[Shaping] = None,
        role_prompts: Optional[dict[str, str]] = None,
    ):
        if reward not in ("pass", "score"):
            raise ValueError("reward is 'pass' or 'score'")
        self.arena = arena
        self.task = task if isinstance(task, Task) else arena.task(str(task))
        self._specs = [a if isinstance(a, Agent) else Agent.external(a) for a in agents]
        self.possible_agents = [a.id for a in self._specs]
        self.agents: list[str] = []
        self.scenario = scenario
        self.max_steps = max_steps
        self.reward = reward
        self.shaping = shaping
        self.role_prompts = role_prompts or {a.id: a.prompt for a in self._specs if a.prompt}
        self.branch: Optional[Branch] = None
        self.steps = 0
        self.check: Optional[CheckResult] = None
        self._last_seq = 0
        self._endpoints: dict[str, AgentEndpoint] = {}

    # -------------------------------------------------------------- spaces

    def observation_space(self, agent: str) -> dict[str, Any]:
        """What an observation holds (a description, not a Gym space)."""
        return {"tool_result": "dict|None", "messages": "list", "world_events": "list"}

    def action_space(self, agent: str) -> list[dict[str, Any]]:
        """The tool calls the agent may make: its tools' schemas (plus :data:`END`)."""
        return [t.__dict__ for t in self._endpoints[agent].tools]

    # -------------------------------------------------------------- episode

    def reset(
        self, seed: Optional[int] = None, options: Optional[dict[str, Any]] = None
    ) -> tuple[dict[str, dict], dict[str, dict]]:
        """Finishes the previous episode if it is still live, starts a new external run of the task
        (with ``seed``), and registers the agents."""
        self._finish_live()
        self.branch = self.arena.external_run(
            self.task, agents=self._specs, scenario=self.scenario, seed=seed
        )
        self._endpoints = dict(self.branch.agents)
        return self._begin(self.branch, history=False)

    def _begin(self, branch: Branch, history: bool) -> tuple[dict[str, dict], dict[str, dict]]:
        self.branch = branch
        self.agents = [a for a in self.possible_agents if a in self._endpoints]
        self.steps = 0
        self.check = None
        rec = branch.record()
        self._last_seq = rec.events[-1].seq if rec.events else 0
        obs, infos = {}, {}
        for a in self.agents:
            ep = self._endpoints[a]
            obs[a] = {
                "prompt": _prompt(self.task, self.role_prompts.get(a, "")),
                "ticket": self.task.ticket,
                "instructions": self.task.instructions,
                "tools": [t.__dict__ for t in ep.tools],
                "tool_result": None,
                "messages": [],
                "world_events": [],
            }
            if history:
                obs[a]["history"] = [e.to_dict() for e in ep.history]
            infos[a] = {"branch_id": branch.id, "seed": rec.seed}
        return obs, infos

    def step(self, actions: dict[str, Any]):
        """Performs each live agent's action and returns ``(observations, rewards, terminations,
        truncations, infos)``."""
        if self.branch is None or not self.agents:
            raise RuntimeError("call reset() first (or the episode has ended)")
        results: dict[str, Any] = {}
        ended: set[str] = set()
        for a in list(self.agents):
            act = actions.get(a)
            if act is None:
                continue
            if act == END:
                self._endpoints[a].end(ok=True)
                ended.add(a)
                continue
            name, args = _parse_action(act)
            r = self._endpoints[a].call(name, args)
            results[a] = {"text": r.text, "is_error": r.is_error, "value": r.value}
        self.steps += 1
        new = self._new_events()
        live = [a for a in self.agents if a not in ended]
        truncated = self.steps >= self.max_steps and bool(live)
        done = not live or truncated
        obs, rewards, infos = {}, {}, {}
        for a in self.agents:
            obs[a] = {
                "tool_result": results.get(a),
                "messages": _messages_for(a, new),
                "world_events": [e.payload for e in new if e.kind == "world_event"],
            }
            rewards[a] = float(self.shaping(self, a, actions.get(a), obs[a])) if self.shaping else 0.0
            infos[a] = {"step": self.steps, "seq": self._last_seq}
        if done:
            if truncated:
                for a in live:
                    self._endpoints[a].end(ok=False, error="truncated")
            self.branch.finish()
            self.check = self.branch.check()
            final = self._terminal_reward()
            for a in self.agents:
                rewards[a] += final
                infos[a]["check"] = self.check
                infos[a]["status"] = (self.branch.final or {}).get("status")
        terminations = {a: (a in ended) or (done and not truncated) for a in self.agents}
        truncations = {a: truncated and a not in ended for a in self.agents}
        self.agents = [] if done else live
        return obs, rewards, terminations, truncations, infos

    def _terminal_reward(self) -> float:
        if self.check is None:
            return 0.0
        return float(self.check.passed) if self.reward == "pass" else self.check.score

    def _new_events(self) -> list[Event]:
        assert self.branch is not None
        events = [e for e in self.branch.events() if e.seq > self._last_seq]
        if events:
            self._last_seq = events[-1].seq
        return events

    def _finish_live(self) -> None:
        if self.branch is not None and self.agents:
            self.branch.finish()
            self.agents = []

    def fork(self, at_seq: int, restore: str = "exact"):
        """A new env on a fork of this (finished) episode at step ``at_seq``: the world as it was just
        before it, each agent's recorded events before that step in ``observations[a]["history"]``.
        Returns ``(env, observations, infos)``; this env is left as it was."""
        if self.branch is None:
            raise RuntimeError("nothing to fork: call reset() first")
        if self.agents:
            raise RuntimeError("finish the episode (step to the end or close()) before forking it")
        forked = self.branch.fork(at_seq, restore=restore)
        env = copy.copy(self)
        env._endpoints = dict(forked.agents)
        obs, infos = env._begin(forked, history=True)
        return env, obs, infos

    def close(self) -> None:
        """Finishes the live episode (scoring it)."""
        self._finish_live()


def _parse_action(act: Any) -> tuple[str, dict[str, Any]]:
    if isinstance(act, dict):
        return act["name"], dict(act.get("arguments") or act.get("args") or {})
    if isinstance(act, (tuple, list)) and len(act) == 2:
        return act[0], dict(act[1] or {})
    raise ValueError(f"an action is {{'name': ..., 'arguments': {{...}}}}, None or END, not {act!r}")


def _messages_for(agent: str, events: list[Event]) -> list[dict[str, Any]]:
    out = []
    for e in events:
        if e.agent == agent:
            continue
        to = e.payload.get("to")
        if e.kind == "message" and to in (agent, "*"):
            out.append({"from": e.agent, "kind": "message", "text": e.payload.get("text")})
        elif e.kind == "handoff" and to in (agent, None):
            out.append({"from": e.agent, "kind": "handoff", "text": e.payload.get("note")})
    return out


def _prompt(task: Task, role: str) -> str:
    parts = [p for p in (role.strip(), f"# Ticket\n\n{task.ticket}", task.instructions) if p]
    return "\n\n".join(parts)


__all__ = ["ArenaParallelEnv", "END"]
