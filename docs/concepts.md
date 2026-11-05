# Concepts

How Arena models a run: the world it runs in, the team that works in it, and how a run is recorded, forked
and scored.

Arena is a staging environment. It observes and never dictates: it knows every agent so that it can monitor,
record, save and rewind it, but it imposes no team shape, order, permission or handoff protocol of its own.
Whatever the design does in production, it does here. Arena is slower than production (it saves the world and
writes the record at every step), so it times its own work and reports what the run would take without it. The protocol each program speaks is in [protocol.md](protocol.md); routes, settings and disk use
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
  `ARENA_CLOCK_SPEED`) while agents work and stands still while the world is saved or restored and while the
  record is written, so time-based world logic sees about the time it would see in production.

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

A team is a description: agents, each with an id, a name, a kind, a model and a prompt. It is not a protocol.
Every agent may call every one of the world's tools, writes included, unless its team marks it `read_only`: then
it is given only the read tools, and a write call is refused and recorded. Who does what is the team's design,
not arena's. `GET /api/team/default` gives the team the benchmark suggests (the retail example suggests a
read-only triage, then a resolver), every agent Claude Code with model `sonnet` unless the query names another
kind, model or provider. A benchmark that suggests none gets one generic agent.

Every agent also gets three tools for talking to the others, all optional and recorded when used:

- `handoff {note, to?}`: a note for the agents that start after it (or for one agent). It ends nothing, and an
  agent may leave several.
- `message {to, text}`: a message to one agent, or to every agent with `to: "*"`.
- `inbox {}`: the notes and messages sent to the agent so far, oldest first.

**Who drives the agents** is the run's runner, chosen when it starts and recorded in its `RunStart`:

- `chain` (the default): arenad runs the agents one after another, in the team's order. Each agent's input is the
  task's ticket, the benchmark's instructions, and the handoff notes left for it so far.
- `concurrent`: arenad starts every agent at once with the ticket and instructions. The branch is scored when all
  of them are done.
- `external`: the design's own code drives the agents (see [External designs](#external-designs)).

In every runner the world freezes (no tick, no clock) once the agents are done, and is then scored.

**Runs recorded before runners** (no `runner` in their `RunStart`) keep the rules they ran under, and so do their
forks: a chain where every agent but the last must call `handoff` with a note, or the branch fails; the last
agent has no `handoff`; an agent's input has only the previous agent's note; and the world freezes before the
last agent's end is recorded.

Runs recorded before `read_only` existed ran under the old rule, where only the last agent could write. A stored
team whose agents carry no `read_only` is read that way (every agent but the last read-only), so those runs, and
forks of them, show and keep the permissions they ran with. Every team stored since records `read_only` for each
agent.

An agent's `kind` picks its mode:

- **CLI mode** (`claude_code` or `codex`): arenad starts the CLI headless and gives it only the world's tools
  (the benchmark's tools plus any tools the parts declare) and the talk tools, through an MCP endpoint of arenad.
  It records every tool call (arguments, result, keys read and written), every note and message, and the tokens
  and cost the CLI reports. It does not see the raw model calls.
- **API mode** (`open`): arenad runs the agent loop itself and sends each model call through its gateway to
  the agent's **provider**. It records everything CLI mode does, plus every model call. Each agent picks one:
  - `openrouter` (the default, also for agents and runs made before providers): needs `OPENROUTER_API_KEY`;
    the model is an OpenRouter model id, `deepseek/deepseek-chat-v3.1` when none is set.
  - `openai`: OpenAI's Chat Completions API; needs `OPENAI_API_KEY` and a model.
  - `anthropic`: Claude through Anthropic's Messages API, translated by the gateway; needs `ANTHROPIC_API_KEY`
    and a model.
  - `local:<name>`: any OpenAI-compatible server (Ollama, vLLM, LM Studio, llama.cpp's server), configured in
    `ARENA_LOCAL_MODELS`, for example `ollama=http://127.0.0.1:11434/v1`; needs a model.

  Every call is recorded in one format (OpenAI chat completions), whatever the provider, so replay, forks and
  compare treat all providers alike. The details, and what each provider costs, are in
  [reference.md](reference.md#model-providers). No API-mode run has been measured yet with any provider.

Isolation of the CLI agents (`core/crates/arenad/src/agents/`; checked live by the `live_` tests in
`core/crates/arenad/tests/cli.rs`):

- Claude Code: settings sources empty, strict MCP config, slash commands off, built-in tools off (`--tools ""`),
  so the only tools are the MCP tools.
- Codex: user config, rules and session storage ignored; apps, plugins, hooks and similar features disabled;
  web search off; read-only sandbox. Codex's built-in shell cannot be turned off. A probe from inside the
  sandbox could not open a socket, not even to the loopback Postgres port (`Operation not permitted`). Codex
  still listed its sub-agent tools (`collaboration.*`) in the live checks; they run under the same restrictions.
- Both CLIs run in a fresh temporary working directory, with `PG*`, `ARENA_PG_URL`, `DATABASE_URL`,
  `OPENROUTER_API_KEY`, `ARENA_LOCAL_MODELS` and every `ARENA_LOCAL_KEY_*` removed from their environment. Each
  keeps only its own vendor's key, which it may sign in with: Claude Code keeps `ANTHROPIC_API_KEY` and loses
  `OPENAI_API_KEY`; Codex keeps `OPENAI_API_KEY` and loses `ANTHROPIC_API_KEY`. The `provider` of API mode does
  not apply to them.
- A CLI session cannot be resumed in the middle, which drives the fork rules below.

## Forking

You choose a step N and a change (prompt, model, provider, kind or `read_only` of one or more agents) and a
restore mode.

- Each part of the world is put back as it was just before the fork point, in the restore mode you choose.
- Agents that finished before the fork point keep their recorded output (for example the handoff note).
- The fork point moves back when the step can't be redone on its own: to the start of the agent when the agent
  active at step N is a CLI agent (a CLI session can't be cut at a tool call) or the change switches its mode;
  to the start of the earliest changed agent when the change is for an agent that finished before step N; and,
  for an API-mode agent, to the model call that chose the tool call at step N. The response and the UI say
  where the fork started (`start_seq`), where you asked for (`moved_from`), and why (`reason`). These rules are
  the chain runner's. A fork of a `concurrent` run restarts every agent from the run's first agent start (agents
  that ran at once can't be cut at one step each). A fork of an `external` run starts where asked: how its agents
  resume is its design's choice ([External designs](#external-designs)).
- A change is never silently ignored.
- If there is no restore point for the step, or a part can't be restored in the mode asked for, the fork is
  refused with an error that names the part.
- Changing `kind` resets the agent's model to the new mode's default, unless the same change also sets a model.
  An empty model in a change clears the model.
- Changing an API-mode agent's `provider` is a model change for these rules. It drops the old model, which
  belongs to the old provider, unless the same change sets one; only OpenRouter has a default model, so the web UI
  always sends a model with a provider. Naming the agent's own provider is no change.
- Changing `read_only` is treated like a prompt change; setting it to what the agent already has is no change.

## External designs

A design that lives in its own code (or in Sculptured) drives a run itself, and arena records it:

1. `POST /api/runs` with `runner: "external"` starts the world and records `RunStart`; the team may be empty.
2. `POST /api/branches/{id}/agents` registers an agent, at any time during the run. The answer gives its MCP
   endpoint (the world's tools and the talk tools, streamable HTTP) and its gateway (an OpenAI-compatible base
   URL whose calls go to the agent's provider). Its kind is `external` unless it says otherwise.
3. The design drives its agents however it likes, through those endpoints. Every tool call, model call, note and
   message is recorded as in arenad's own runners. A CLI agent's model calls don't pass through the gateway, so
   the design reports each one its client saw (`POST .../agents/{agent}/usage`: the call's tokens, cache tokens
   included, and the model's context window; or a compaction of its context), and arena records it as a
   `model_call` without request or response. `POST .../agents/{agent}/end` records that an agent is done, with
   the tokens and cost the design reports for it. `GET /api/tasks` gives the ticket and the instructions to give
   the agents.
4. `POST /api/branches/{id}/finish` freezes, scores and stops the world.

A fork of an external run starts at the step asked for, restores the world as it was just before it, and leaves
the new branch live. Its answer gives each agent's recorded history before the fork point and its endpoints on the
new branch, so the design resumes its agents its own way. A branch whose design never calls `finish` stays running
until arenad restarts (it is then `interrupted`).

## Overhead and estimates

On every step arena times its own work apart from the agents' and the world's. Each recorded event carries
`overhead_ms`, the time arena spent since the branch's previous event, per kind: `restore` (starting the world from
a save or by replay), `save`, `record` (writing the record), `gateway` and `mcp` (the endpoints' own handling,
besides the provider's answer and the tool's run), `gate` (gate checks), `link` (link calls a step set off),
`wait` (waiting for the branch's step lock, which keeps steps and saves in one order) and `score` (the final saves
and scoring). The time spent writing an event is counted on the next one; the last write of a branch is not
counted.

A branch's metrics (`GET /api/branches/{id}`, compare, and the run page) count only its own steps (`seq >=
start_seq`), as compare does:

- **measured**: wall time from the branch's creation to its end;
- **estimated in production**: measured minus the overhead, with the breakdown per kind. When agents run at once
  their overheads may overlap in time, and the sum can then exceed what they added to the wall time, so the
  estimate is a lower bound. Runs recorded before overhead was timed have no estimate;
- **tokens** per agent and model (input and output where they are known), and per agent its model calls, the most
  context one was sent with (`peak_prompt_tokens`), the compactions its client reported, and the overhead recorded
  on its own events (`overhead_s`; when agents run at once, an event's overhead may have held up others too);
- **cost** per agent: the cost the provider reported (`reported`; Claude Code's own figure is an API-equivalent
  estimate); `no API charge` for a local model; otherwise tokens times the list price in `docs/prices.toml`,
  labelled `list-price estimate` with the price's source and date (base input and output prices only: caching,
  batch and long-context prices are not modelled); otherwise `unknown`. Arena never makes up a price. The branch's
  cost is the sum when every agent's is known (labelled `mixed` when their labels differ), else `unknown`.

## Cost labels

- `openrouter`: the cost OpenRouter reports (API mode on OpenRouter).
- `local (no API charge)`: API mode on a local server; the cost is 0 and the UI shows `no API charge`.
- No label and no dollar figure on API mode with OpenAI or Anthropic: they report no price and arenad keeps no
  price table, so only tokens are recorded; the UI shows the cost as `unknown`.
- These labels are the providers' own. The metrics above also estimate a cost from `docs/prices.toml` where no
  provider reported one.
- `api_equivalent_estimate`: Claude Code's own `total_cost_usd`. It is what the same tokens would cost at API
  prices, not what a subscription pays.
- No label and no dollar figure: Codex reports tokens only.
- Dollars are shown only when every agent in the branch has a dollar figure; otherwise only tokens are shown,
  and the UI writes the cost as `unknown` when an agent of the branch is on OpenAI or Anthropic (else `tokens
  only`). Tokens are always summed. When agents have different sources the label is `mixed`.
