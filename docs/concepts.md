# Concepts

How Arena models a run: the world it runs in, the team that works in it, and how a run is recorded, forked
and scored. The protocol each program speaks is in [protocol.md](protocol.md); routes, settings and disk use
are in [reference.md](reference.md).

## Glossary

- **Integration**: a program, in any language, that puts one kind of system in the world (a database, a folder
  of files, a Docker Compose project, a simulator). It starts instances, runs tool calls, saves an instance's
  state, starts from a save, shows the state (`inspect`) and compares two states (`diff`). In `describe` it
  declares its tools, how it can restore, and whether it uses the arena clock.
- **Part**: one instance of an integration in a world, with a name and a config.
- **World**: the list of parts a benchmark names. Tool names must be unique across a world.
- **Benchmark**: a package that names a world and brings tasks, instructions for the agents, tools, the data
  that seeds the world, and for each task the gold actions that define the expected result.
- **Run**: one task, one team, one world. A run has one or more branches; its first branch is the original.
- **Step**: one recorded event of a branch (an agent start, a model call, a tool call, a handoff, ...),
  numbered by `seq`.
- **Save**: after a step that wrote to a part, and for every clocked part whose state changed, arenad saves
  those parts and records the save ids with the step. Parts that can only replay are never saved.
- **Fork**: a new branch of a run that starts at step N of an existing branch with a change (an agent's prompt,
  model or kind). The world is restored to how it was just before the fork point, and the branch reruns from
  there.
- **Restore**: a fork restores the world exactly or approximately (see [Restore modes](#restore-modes)).
- **Scoring**: when a branch ends, arenad saves every part and diffs the world with the task's gold world (the
  base world plus the task's gold actions, made once and kept). The branch passes when no key differs and at
  least one part could be checked.
- **Clock**: parts with `uses_clock` get the arena clock, which runs with real time (times
  `ARENA_CLOCK_SPEED`) while agents work and stands still while the world is saved or restored.

## Restore modes

A fork asks for one restore mode for the whole world. Each integration declares what it can do in `describe`:

- **exact** (`restore.exact`: `save`, `replay` or `none`): the part is exactly as it was. From an exact save
  when the part has one (fast), or by replaying from the start with the same seed and every recorded input up to
  the step (slower the later the step). If a replay gives a different result than the recorded one, the fork
  fails with "replay of <part> diverged at step N, so it can't be restored exactly". A replay is refused when
  the part's fingerprint (versions, source hashes, ...) changed.
- **approximate** (`restore.approximate`: true or false): from a save that may not capture everything (fast).
  Branches restored this way are labelled "Approximate restore" in the run page, the branch list and compare.

| Part declares | exact | approximate |
|---|---|---|
| exact save | from its save | same |
| replay and approximate | replay | from its save, approximately |
| replay only | replay | replay |
| approximate only | refused, naming the part | from its save, approximately |
| none | refused | refused |

The table is implemented in `core/crates/arenad/src/restore.rs`. `GET /api/branches/{id}/restore-plan` and the
rerun panel show, per part, how it will be restored and, for a replay, an estimate of the time.

## Teams and agents

A team is a list of agents that run one after another. Each agent's input is the task's ticket, the benchmark's
instructions, and the previous agent's handoff note. Every agent sees the world's tools; only the last agent may
call write tools (the others are refused), and every agent but the last must call `handoff` with a note for the
next one, or the branch fails. `GET /api/team/default` gives the team the benchmark suggests, every agent
Claude Code with model `sonnet` (the retail example suggests triage, then resolver). A benchmark that suggests
none gets one generic agent.

An agent's `kind` picks its mode:

- **CLI mode** (`claude_code` or `codex`): arenad starts the CLI headless and gives it only the world's tools
  (the benchmark's tools plus any tools the parts declare) and `handoff`, through an MCP endpoint of arenad. It
  records every tool call (arguments, result, keys read and written), the handoff note, and the tokens and
  cost the CLI reports. It does not see the raw model calls.
- **API mode** (`open`): arenad runs the agent loop itself and sends each model call through its gateway to
  OpenRouter. It records everything CLI mode does, plus every model call. It needs `OPENROUTER_API_KEY`. The
  model is an OpenRouter model id, `deepseek/deepseek-chat-v3.1` when none is set. No API-mode run has been
  measured yet.

Isolation of the CLI agents (`core/crates/arenad/src/agents/`; checked live by the `live_` tests in
`core/crates/arenad/tests/cli.rs`):

- Claude Code: settings sources empty, strict MCP config, slash commands off, built-in tools off (`--tools ""`),
  so the only tools are the MCP tools.
- Codex: user config, rules and session storage ignored; apps, plugins, hooks and similar features disabled;
  web search off; read-only sandbox. Codex's built-in shell cannot be turned off. A probe from inside the
  sandbox could not open a socket, not even to the loopback Postgres port (`Operation not permitted`). Codex
  still listed its sub-agent tools (`collaboration.*`) in the live checks; they run under the same restrictions.
- Both CLIs run with `PG*`, `ARENA_PG_URL`, `DATABASE_URL` and `OPENROUTER_API_KEY` removed from their
  environment, in a fresh temporary working directory.
- A CLI session cannot be resumed in the middle, which drives the fork rules below.

## Forking

You choose a step N and a change (prompt, model, or kind of one or more agents) and a restore mode.

- Each part of the world is put back as it was just before the fork point, in the restore mode you choose.
- Agents that finished before the fork point keep their recorded output (for example the handoff note).
- The fork point moves back when the step can't be redone on its own: to the start of the agent when the agent
  active at step N is a CLI agent (a CLI session can't be cut at a tool call) or the change switches its mode;
  to the start of the earliest changed agent when the change is for an agent that finished before step N; and,
  for an API-mode agent, to the model call that chose the tool call at step N. The response and the UI say
  where the fork started (`start_seq`), where you asked for (`moved_from`), and why (`reason`).
- A change is never silently ignored.
- If there is no restore point for the step, or a part can't be restored in the mode asked for, the fork is
  refused with an error that names the part.
- Changing `kind` resets the agent's model to the new mode's default, unless the same change also sets a model.
  An empty model in a change clears the model.

## Cost labels

- `openrouter`: the cost OpenRouter reports (API mode).
- `api_equivalent_estimate`: Claude Code's own `total_cost_usd`. It is what the same tokens would cost at API
  prices, not what a subscription pays.
- No label and no dollar figure: Codex reports tokens only.
- Dollars are shown only when every agent in the branch has a dollar figure; otherwise only tokens are shown.
  Tokens are always summed. When agents have different sources the label is `mixed`.
