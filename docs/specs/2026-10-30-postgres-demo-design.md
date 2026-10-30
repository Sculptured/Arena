# Reversible arena: basic Postgres demo (design)

Status: draft for approval, 2026-10-30.

## Intent

The first working version of the platform in the CS2680 proposal
(`plans/cs2680-proposal/write-your-own/proposal.tex`). A user runs a small team of agents against a
Postgres database. The platform records every step with the database state. From the UI the user then
goes back to any step, changes a component (prompt, model, or which harness runs the agent), and reruns
from there with the database restored. Then they compare the two branches.

The Ridgeline factory video is the anti-example: eight agents, ERPNext, a plant simulator, Isaac and a
video edit. That is too much to follow. This demo is one screen, two agents and one database.

## Goals

- Run a triage → resolver team on τ²-bench retail tasks, with the retail data in Postgres.
- Record each step: model calls (when we can see them), tool calls with the rows they read and wrote,
  handoffs, and the score at the end.
- Snapshot the database after every write, so restoring to any step is cheap.
- Fork a run at a step, with changes, and rerun from there. Compare branches on outcome, cost, time,
  database differences, and the first step where they diverge.
- Two agent modes, switchable per agent:
  - **CLI:** Claude Code or Codex, with the user's existing sign-in. There is no API key, and we don't
    see their model calls.
  - **Open:** our own agent loop calling OpenRouter through our recording gateway. Every model call is
    captured. It needs `OPENROUTER_API_KEY` and is disabled without it.
- The core is a Rust service. The UI is a separate web app that talks to it only over HTTP.

## Non-goals (later)

- Fault injection.
- SGLang or vLLM hosting, serving-state capture and chain-of-thought capture.
- Dashboards.
- Benchmarks other than τ²-bench retail.
- Fault search and design search.
- Integration with Sculptured.

## Anti-goals

- A demo that needs more than one screen to follow.
- Claiming a fork restores something it doesn't.
- Fabricated or synthetic results shown as measured.

## Layout

```
arena/
  core/                     Rust workspace
    crates/world/           Postgres world: retail schema, loader, tools, read/write sets, snapshots, check
    crates/arenad/          the service: HTTP API, MCP endpoint, model gateway, runner, fork, compare
  ui/                       React 19 + Vite + TypeScript, styled from Sculptured's web app
  data/retail/              τ²-bench retail db.json, tasks.json, policy.md (MIT, with attribution)
  deploy/compose.yml        Postgres 17 on port 5544
  docs/specs/               this file
```

Rust dependencies follow Sculptured's choices where they overlap: axum 0.8, tokio, serde, reqwest, and
tokio-postgres for Postgres.

## Core

**Postgres.** One server holds several databases:
- `arena` for runs, branches and events;
- `world_base` for the retail data, loaded once;
- `snap_<id>` for snapshots;
- `branch_<id>` for each branch's live database.

The retail tables are `users`, `orders` and `products`. The nested parts (addresses, payment methods,
items, fulfillments, variants) are JSONB columns. Each row is keyed by its τ²-bench id.

**Tools.** A port of `cs2680-gap-tests/harness/tool.py` (itself a port of τ²-bench's retail toolkit)
to SQL. Each call returns its result and the set of rows it read and wrote (`table:key`). The port is
correct when it reproduces the gold database of all 68 eligible tasks, the same test `gap.py select`
runs. The triage agent's writes are refused.

**Snapshots.** After every write that succeeds, the core runs `CREATE DATABASE snap_x TEMPLATE
branch_y`. A branch is created the same way from the nearest snapshot. The core holds no open
connection to a branch while copying it. The retail data is about 3 MB, so each copy should be quick.
We measure this rather than assume it.

**Recording.** The `arena` database holds an append-only event table. Each row has a branch, a sequence
number, an agent, a type and a JSON payload. The event types:
- run start;
- agent start and end;
- model call (the request, response, tokens and cost);
- tool call (arguments, result, read and write sets, and the snapshot id if it wrote);
- handoff note;
- check result.

**Agents.**
- **Claude Code:** `claude -p` with an inline `--mcp-config` pointing at
  `/mcp/<branch>/<agent>` on arenad, `--strict-mcp-config`, and only the retail MCP tools allowed.
- **Codex:** `codex exec` with the same MCP server set through `-c mcp_servers...`. The installed
  version's support for HTTP MCP servers gets checked first. If it lacks it, a small stdio bridge
  forwards to arenad.
- **Open:** a loop in arenad that sends OpenAI-style chat completions with tools to
  `/gateway/<branch>/<agent>/v1/chat/completions`. That endpoint records the call and forwards it to
  OpenRouter. Any OpenAI-compatible agent can use the same endpoint later.
- **Team shape** (my call): a chain of agents, each handing off to the next. The default is triage →
  resolver. Agents can be added, removed and reordered, but there is no branching yet. Only the last
  agent may write.
- **Handoff:** triage ends by calling a `handoff(note)` tool. The resolver starts with the ticket, the
  policy and that note. The prompts come from `gap.py`, adapted to tools instead of files.

**Score.** τ²-bench's database check. The final branch database must equal the database produced by
replaying the task's gold actions on a copy of `world_base`. The comparison uses a canonical dump of the
tables.

**Fork.** `POST /api/runs/{id}/fork {at_seq, changes: {agent: {prompt?, model?, kind?}}}`. Going
back to step N means **step N is redone with the change** (my call): the world is as it was just before
step N.
- The new branch's database is copied from the last snapshot taken before step N.
- Agents that finished before step N keep their recorded output, such as the handoff note.
- Open-mode agent active at step N: its recorded conversation is replayed up to just before that step,
  and it then continues live.
- **The fork point moves back** to an agent's start in three cases (my call):
  - the agent active at step N runs in CLI mode, because a CLI session can't be cut at an arbitrary
    tool call;
  - its mode changes;
  - a change targets an agent that finished before step N. In that case the fork point moves to the
    start of the earliest changed agent.

  The fork never silently ignores a change. The response and the UI state where it actually started
  and why.

**Cost** (my call). Every cost is labeled with where it came from:
- **Open mode:** the cost OpenRouter reports.
- **Claude Code:** its `total_cost_usd`, labeled "API-equivalent estimate", since a subscription pays
  something else.
- **Codex:** tokens only.

Compare shows tokens for every mode, and dollars only where a dollar figure exists.

**API.** JSON over HTTP, with live events over SSE:
- teams;
- tasks;
- runs (create, list, read, events);
- fork;
- compare: outcome, cost, time, database row differences, and the first divergent step.

## UI

A separate Vite app (`arena/ui`), run with its own dev server or built to static files that arenad can
serve. It copies Sculptured's design tokens and fonts, and follows its layout: a top bar, a header with
tabs, and the dark board look. Pages:

1. **Runs:** the list of runs and their branches, plus "New run" (pick a task and a team).
2. **Team:** the agents, each with a name, mode (Claude Code, Codex or Open), model and prompt.
3. **Run:**
   - a timeline with one lane per agent, showing model calls, tool calls, handoffs and snapshot markers;
   - a step inspector showing arguments, result, the rows read and written, and the rows as they were at
     that step;
   - a "Change and rerun from here" panel;
   - the branch tree on the side.
4. **Compare:** two branches side by side, with outcome, cost, time, database differences, and the first
   divergent step.

## Tests

- The tool port reproduces the gold database on all 68 eligible tasks.
- A snapshot and restore round trip gives the same canonical dump.
- A fork test with a scripted agent, so no model calls are needed. It checks:
  - the forked branch sees the database as it was at the fork step, not as the first run left it;
  - recorded outputs are reused;
  - the change takes effect.
- One live run each with Claude Code and with Codex on one task, recorded in the README with what it
  cost.
- The UI type-checks and builds.

## Separate fix: board layout of imported groups (Sculptured)

When the Ridgeline team file is imported, the groups come in with the right data: the same 5 groups and 8
agents, at the same positions. The board draws them wrong, though:
- agent cards spill out of their group boxes;
- group headers sit on top of cards;
- the Incident group is a full-width box holding one card.

**Cause** (found 2026-10-30):
- The web app grows a group to fit its agents' cards (`fitGroups`, `web/src/draft/edits.ts`), but only
  when you edit on the board.
- A team that arrives through the API, as `install_team.py` imports it, keeps the box sizes from the
  file.
- The Ridgeline file's groups are 260 px tall, but a card is 200 px (`CARD_H`) and starts 72 px into its
  group, so every card sticks out of its box.

**Fix:** when the board loads a draft whose groups don't contain their agents, it applies `fitGroups`
for display. It also saves the fitted layout the next time the user edits. A test imports the Ridgeline
file and checks that every card is inside its group and no groups overlap.

Separately, the factory's `build_team.py` gets sizes that fit the cards, and the Incident group shrinks
to its one agent.

## Left to the builder

- Crate-internal structure.
- Exact table columns.
- UI component breakdown.
- Port numbers other than Postgres.
