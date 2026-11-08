# A dispatching team runner, and the single agent as the default (2026-11-07)

Approved by the user in chat on 2026-11-07. Their words, about the default 6-agent team on the `chain` runner:
"the runs being sequential means its a bad design or problem, try with one agent, like its just bad seeing it
perform so poorly. because its sequentially used agents doing a single task, no dispatching and continued
working." They chose: "Single agent default + build dispatch".

Evidence: on the pharma task `cvx-2611-em-excursion`, the single agent (Claude Code, Sonnet) passed 13 of 14
checks, all 5 safety checks, for about $0.47; the 6-agent `chain` team (Claude Code, Haiku) passed 8 of 14.

## Hard rule
Real models only: no scripted, demo, stub or fake agents anywhere (code, tests, fixtures). Agent tests are live
on a real model (Claude Code on haiku by default, see core/crates/arenad/tests/support/live.rs) and tolerate
model variance.

## 1. The single agent is the default
- A benchmark's suggested team is now its single agent where it has one (pharma: `data/teams.json`
  deployment `single`; factory: add one if it lacks it, same pattern). The Run screen opens with it; a browser
  that stored the old coordinated team as an untouched default switches to the new default (an edited or
  imported team stays as the user left it).
- The other suggested teams stay one click away: the Team panel offers the benchmark's teams (single agent,
  dispatching team) as a small picker next to Import/Edit.
- The plain `chain` runner moves under Advanced (still available, never deleted).

## 2. A `dispatch` runner (a new built-in runner, beside `chain`, `concurrent`, `external`)
- **A lead and specialists.** The team's first agent (or the one marked `lead`) is the lead. It starts with the
  ticket and the instructions, and it alone ends the run.
- **Delegation.** The lead assigns work with a talk tool (`delegate {to, task}`; it may also use `message`).
  Every specialist starts idle and wakes when work or a message arrives.
- **Continued working.** Agents stay alive across turns: a specialist that finishes a turn waits (it isn't done)
  and wakes on the next delegation or message; it reports back with `report {text}` (to the lead) or
  `message`. CLI agents keep one long-lived session (Claude Code `-p` with stream-json in/out, as
  Arena-Examples/factory-showcase/designs/claude_agent.py does, with arenad's own isolation flags); API-mode
  agents keep their conversation in arenad's loop.
- **Parallel.** Specialists work at the same time; arenad serialises world steps as it does today (the step
  lock), so the record stays one ordered sequence.
- **Ending.** The lead calls `finish {summary}` (a talk tool only the lead has); then every agent is stopped,
  the world freezes and is scored. Guard rails: a run-level time limit (configurable; default generous), and an
  idle rule (if every agent is idle and the lead hasn't finished after a grace period, wake the lead with the
  state of the team; after N such wakes, end the run as failed with a clear reason).
- **Recorded.** Delegations, reports and wakes are events, so the timeline shows dispatching (connectors from
  the lead to each specialist and back) and the step drawer reads them in words. Forks: a fork of a dispatch
  run restarts every agent from the fork point with its recorded history (as for `concurrent`, document the
  exact rule), pause/resume/stop work as for the other runners.
- **Prompts.** The benchmark's dispatching team (pharma: from the coordinated roles) gets role prompts written
  for this runner: the lead plans, delegates, checks reports, follows up and finishes; specialists do their
  part, report, and stay available. No handoff-only chain wording.
- Generalise from the factory showcase's hierarchical design (Arena-Examples/factory-showcase/designs/engine.py,
  designs.py: delegation by message, waking idle agents, end conditions); don't depend on Arena-Examples at
  runtime.

## 3. UI
- Runner choice under Advanced gains "A lead dispatches to the team (dispatch)", and the dispatching team in the
  Team picker selects it automatically.
- The live timeline shows delegations/reports as connectors, and agents' idle periods as faint gaps.

## Verification
Unit tests for the runner's state machine (delegation, wake, idle rule, finish, stop, pause, fork restart) on
recorded/constructed events with no agent; live tests (haiku) that a dispatch run delegates, specialists work in
parallel, report, and the lead finishes. Then real runs on the pharma task with Claude Code Sonnet: the single
agent (default) and the dispatching team, compared in Arena (outcomes table). Report the checks, cost and time
of each. Keep real runs to what's needed.
