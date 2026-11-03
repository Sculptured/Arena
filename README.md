# Arena

A rewindable arena for testing multi-agent systems.

A team of AI agents works on a task in a world. The world is made of integrations: isolated programs, like
device drivers, that each put one kind of system in the world and speak the `arena-part/1` protocol with the
arena service, arenad (`docs/protocol.md`). arenad records every step and saves the world as it was at that
step. You can go back to any step, change an agent's prompt, model or harness, fork with an exact or
approximate restore of the world, rerun from there, and compare the runs.

Benchmarks are separate packages: they bring the tasks, the tools, the data and the scoring, and name the
world they run in. The core (`core/`) does not depend on any integration or benchmark;
`core/crates/parts/tests/isolation.rs` checks this.

Docs: `docs/protocol.md` (the protocol), `integrations/README.md` (how to write an integration),
`docs/specs/2026-10-31-integrations-design.md` (the design), `docs/measurements.md` (measurements made with
the retail example benchmark).

## Concepts

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
- **Restore**: a fork restores the world exactly or approximately (see "Restore modes").
- **Scoring**: when a branch ends, arenad saves every part and diffs the world with the task's gold world (the
  base world plus the task's gold actions, made once and kept). The branch passes when no key differs and at
  least one part could be checked.
- **Clock**: parts with `uses_clock` get the arena clock, which runs with real time (times
  `ARENA_CLOCK_SPEED`) while agents work and stands still while the world is saved or restored.

### Restore modes

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

## Quick start

This uses the included retail example benchmark (`benchmarks/retail`, customer service from tau2-bench retail,
on one `postgres` part), the default benchmark. Needs Docker, Rust and Node; the `claude` and/or `codex` CLIs
on your PATH for CLI agents; `OPENROUTER_API_KEY` for API agents; `uv` only for the Python integrations.
From this folder:

    docker compose -f deploy/compose.yml up -d     # Postgres on 127.0.0.1:5544 (loopback only)
    cargo build --release                          # the Rust integrations and benchmarks, into core/target/release
    (cd core && cargo run -p arenad)               # API on http://127.0.0.1:4510
    (cd ui && npm install && npm run dev)          # UI on http://127.0.0.1:4520 (proxies /api to 4510)

arenad keeps its records in the database `arena` on that Postgres. It starts the integration and benchmark
programs itself, from `ARENA_BIN_DIR` (default `core/target/release`), and refuses to start if the benchmark
can't be described (package missing or not built). It listens on 127.0.0.1 only. `GET /api/health` shows the
benchmark, which CLIs were found, whether API mode is available, and the disk use.

Or build the UI and let arenad serve it:

    (cd ui && npm install && npm run build)
    (cd core && ARENA_UI_DIST=../ui/dist cargo run -p arenad)    # UI and API on :4510

Then:

1. Start a run: pick a task and a team.
2. Click a step to see its arguments, its result, and the state it read or wrote, as it was at that step.
3. Fork at a step with a change (for example a new prompt for one agent, or that agent run by another
   harness). The world is restored to just before that step, and the run continues from there.
4. Compare the two branches: outcome, time, tokens, cost, the first step where they differ, and the state that
   differs at the end.

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

## Integrations

Columns: **Exact** is `restore.exact`, **Approx.** is `restore.approximate`, **Clock** is `uses_clock`, all
from each program's `describe`. **Verified** says what has run, and on what. A "live" test is `#[ignore]`d with
the prefix `live_` and needs Docker or a real server. Each folder's README has the details.

| Name | What it puts in the world | Exact | Approx. | Clock | Verified |
|---|---|---|---|---|---|
| `postgres` | A Postgres database per instance | save | no | no | Tests against the `postgres:17-alpine` server of `deploy/compose.yml`, with the `arena-part-check` run (`integrations/postgres/tests/part.rs`); the retail example runs on it through arenad |
| `folder` | A folder of files per instance | save | no | no | Tests on local disk with the `arena-part-check` run (`integrations/folder/tests/part.rs`); no service involved |
| `service` | Any Docker Compose project per instance | save | no | no | `live_service_roundtrip` against a real Redis 6.2 project in Docker. No `arena-part-check` run |
| `mariadb` | A MariaDB database per instance (saves are binary-log positions) | save | no | no | Seven `live_mariadb_*` tests against MariaDB 11.8 in Docker, including `arena-part-check` |
| `erpnext` | An ERPNext site per instance (saves are binary-log positions plus the site's files) | save | no | no | `live_erpnext_roundtrip` against ERPNext v16.53.0 in Docker, including `arena-part-check` |
| `sculptured` | A Sculptured server per instance, for approvals | save | no | no | `live_sculptured_roundtrip` against a real `sculptured` build; `arena-part-check` passed once by hand |
| `ridgeline-hub` | A factory's simulated suppliers, carriers, insurer and customers | save | no | yes | Python tests against the real hub `market.py`, with `arena-part-check`. No services, no arenad run |
| `isaac-sim` | A factory plant, on Isaac Sim physics or a stand-in | replay | yes | yes | **Stand-in only.** Python tests with `arena-part-check`. The `isaac` backend has never run on Isaac Sim |
| `isaac-sim-mirror` | A read-only view of a running plant | none | no | yes | One Python test against a stub HTTP server. A world containing it can't be rerun |
| `counter` (Rust), `counter-py` (Python) | Kit examples: one integer per instance | save (flavours: replay, approximate) | no (flavour) | yes | Kit and core tests (`kits/*/tests`, `core/crates/parts/tests`, arenad tests) |

Only `postgres` (through the retail example) has been used in an arenad run with agents. The other
integrations are checked on their own, not inside a team's run.

### Write an integration

A folder with an `arena-package.toml` (`kind = "integration"`, `name`, `version`, `protocol`, `run`, `needs`,
`config_schema`, and `[[example]]` tool calls) and a program that speaks the protocol. The Rust kit
(`kits/rust`, crate `arena-part`) and the Python kit (`kits/python`, package `arena_part`) read and write the
lines and check the ids; you fill in `describe`, `start`, `call_tool`, `advance`, `save`, `inspect`, `diff`,
`drop_save`, `stop` and `usage` (`gc` and `connect` are optional). `integrations/README.md` walks through the
counter example (`kits/rust/examples/counter.rs`, `kits/python/examples/counter.py`). Then check it, from this
folder:

    cargo build                                                  # your integration (debug build)
    (cd core && cargo build -p parts --bin arena-part-check)     # the checker
    ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/<name> [--config file.json]

`arena-part-check` runs the program and checks describe, start, the examples, save and restore, diff, replay
and `drop_save`. It prints a JSON report and exits 0 if all passed, 1 if not, 2 if it could not run.

Install a package by putting its folder under one of the `ARENA_PACKAGES` roots. A `command` run is for
programs you trust; it runs in the package folder with only `PATH`, `HOME`, `ARENA_PG_URL`, `UV_CACHE_DIR`,
`XDG_CACHE_HOME`, `RUST_LOG` and `ARENA_SAVES` in its environment. An `image` run is boxed in a container: no
network unless the manifest asks for host networking (`needs.network = "host"`), no environment from arenad,
and only its own saves folder, mounted at `/saves`.

### Write a benchmark

A benchmark is a package with `kind = "benchmark"` in its `arena-package.toml` and a program that speaks the same
transport (`docs/protocol.md`, "Benchmark calls"). It answers:

- `describe`: its name and version, the world (`[{part, integration, config}]`), the tasks (`[{id, reason,
  ticket}]`), the instructions every agent gets, and its tools (MCP shape plus `effect: "read"|"write"`).
  Optionally `instructions_title`, the heading over the instructions in each agent's input (default
  `Instructions`), and `team`, the team it suggests (`{id, name, agents: [{id, name, prompt}]}`).
  The CLIs know the world's MCP server by the benchmark's name.
- `seed`: load the starting data into fresh instances. arenad saves the result once as the base world that
  every run starts from.
- `call_tool`: run one of its tools and say which part it touched and which keys it read and wrote.
- `gold_actions`: the tool calls that solve a task. arenad applies them to the base world once per task to make
  the gold world that branches are scored against.

A benchmark's tools reach parts only through the parts' `connect` info (for `postgres`, `{url}`), so only parts
with exact saves may be connected. In Rust, implement `arena_part::Benchmark` and end `main` with
`serve_benchmark(..)`; the Python kit has no benchmark support yet.

`benchmarks/retail` is the included example: a Rust program (`src/main.rs`, `src/lib.rs`) whose world is one
`postgres` part named `retail`; `src/load.rs` seeds it from `data/retail/db.json`, `src/tools/` holds the tools,
and `src/tasks.rs` reads the tasks and gold actions from `data/retail/tasks.json` (the tasks offered are the
ids in `data/retail/demo_tasks.json`); `src/team.rs` holds the team it suggests and the `Store policy` heading
over its policy. Its tests are in `benchmarks/retail/tests`. arenad runs one benchmark at a time, chosen with
`ARENA_BENCHMARK`.

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

## HTTP API

All JSON on `http://127.0.0.1:4510`, except the event stream. Errors are `{error}` with a status code.

| Route | What it does |
|---|---|
| `GET /api/health` | `{open_available, claude, codex, benchmark, disk: {bytes, limit_bytes}}` |
| `GET /api/tasks` | the benchmark's tasks, `[{id, reason}]` |
| `GET /api/team/default` | the default team |
| `GET /api/runs` | the runs with their branches |
| `POST /api/runs` | `{task_id, team}`: start a run; returns `{run_id, branch_id}` |
| `DELETE /api/runs/{id}` | delete a run and what it saved (409 while a branch runs) |
| `GET /api/branches/{id}` | a branch with its events |
| `GET /api/branches/{id}/stream` | Server-Sent Events: each `event`, then the final `status` |
| `GET /api/branches/{id}/rows?seq=N&keys=<kind>/<id>,...[&part=]` | the state of those keys just after step N (at most 50 keys) |
| `POST /api/branches/{id}/fork` | `{at_seq, changes: {<agent id>: {prompt?, model?, kind?}}, restore: "exact"\|"approximate"}` (default exact); returns `{branch_id, start_seq, moved_from, reason}` |
| `GET /api/branches/{id}/restore-plan?at_seq=N&restore=exact\|approximate` | `{parts: [{part, how, estimate_seconds?}]}`, or 409 with the refusal |
| `GET /api/compare?a=<branch>&b=<branch>` | `{a, b, first_divergent_seq, differing_rows, not_compared}` |
| `GET /api/world` | the benchmark's parts with what each integration reports (version, restore, clock use, tool names) and whether it answered (each gets 5 s; a silent one shows `did not answer within 5 s`) |
| `GET /api/usage` | `{integrations: [{integration, parts, bytes}], total_bytes, limit_bytes}`: the bytes each integration keeps and the world parts that use it |
| `POST /api/gc` | runs the startup cleanup and returns `{removed, freed_bytes, errors}`; it keeps every save a recorded run names and the shared base and gold saves; 409 while agents are running, and new runs and forks answer 409 while it runs |

New runs and forks answer 507 once the saves reach the disk limit. The `/api` routes answer only
requests whose `Host` is a loopback address. The agents' MCP endpoints (`/mcp/{branch}/{agent}`) and the
API-mode gateway (`/gateway/{branch}/{agent}/v1/chat/completions`) are for arenad's own agents.

## Configuration

Environment variables, read once at startup by arenad:

- `ARENA_PG_URL`: the Postgres server for arenad's records, passed on to integration programs (default
  `postgres://arena:arena@127.0.0.1:5544/postgres`).
- `ARENA_PORT`: HTTP port (default 4510).
- `ARENA_BENCHMARK`: the benchmark new runs use, by package name (default: the only installed benchmark;
  required when several are installed).
- `ARENA_PACKAGES`: `:`-separated folders whose subfolders hold an `arena-package.toml` (default
  `integrations:benchmarks` in this folder).
- `ARENA_BIN_DIR`: what `${ARENA_BIN_DIR}` in a manifest stands for (default `core/target/release`).
- `ARENA_SAVES`: where saves are kept (default `core/saves`, ignored by git). Integrations that run in Docker or
  on their own servers also keep data elsewhere.
- `ARENA_MAX_DB_BYTES`: disk limit, see below (default 5 GiB, `5368709120`).
- `ARENA_CLOCK_SPEED`: speed of the world clock (default 1, real time).
- `OPENROUTER_API_KEY`: turns on API mode. Pick the model per agent on the Team page (or per fork in "Change and
  rerun") by typing an OpenRouter model id.
- `ARENA_UI_DIST`: a built UI folder for arenad to serve at `/`.

Default paths are relative to this folder, fixed when arenad is built, so they hold wherever arenad is started
from. The retail example reads its data from `data/retail` (relative to its package folder, `../../data/retail`);
`ARENA_RETAIL_DATA` overrides that when you run its program directly, but arenad does not pass it on.

## Disk

What uses space: every save of every part, and the databases or files a part keeps outside `ARENA_SAVES`. How
big a save is depends on the integration (see its README): a `postgres` save is a full copy of the database
(about 21 MB for the retail example), a `mariadb` save is a binary-log position. Runs and forks add up until you
delete the run. The Rust build directory `core/target` also grows; `cd core && cargo clean` frees it.

- The top bar shows `Saves <used> / <limit> GB`: each integration's `usage` plus the size of `ARENA_SAVES`.
  When it reaches the limit, new runs and forks are refused (HTTP 507) until you delete old runs. The limit is
  `ARENA_MAX_DB_BYTES`.
- Delete a run from the Runs page (Delete run, then confirm), or
  `curl -X DELETE http://127.0.0.1:4510/api/runs/<run_id>`. This drops its saves, stops its live instances and
  removes its recorded events. The shared base and gold saves stay. A run with a running branch can't be
  deleted.
- At startup arenad asks each integration of the benchmark's world to drop saves that no recorded event names
  (left by crashes), and logs `gc: dropped N saves (X MB)`. An integration without `gc` is skipped.
  `POST /api/gc` runs the same cleanup. Start arenad only when no test is running, since tests make
  unreferenced saves.
- The tests drop what they create, even when an assertion fails. `core/scripts/leakcheck.sh` (run from `core`)
  runs each of the core's test binaries and fails if one leaves databases behind: Postgres `pi_`, `ps_`,
  `branch_`, `snap_` and `arena_test` databases, and MariaDB `mi_` and `mx_` ones when `LEAKCHECK_MARIADB` names
  the server container.

## Tests

    (cd core && cargo test)                        # needs the Postgres above
    cargo test                                     # in this folder: kit, integrations, benchmarks (the Postgres above for some)
    cargo test -- --ignored live_                  # live integration tests: they start Docker containers or real servers
    (cd integrations/ridgeline-hub && uv run --with pytest pytest -q)    # likewise integrations/isaac-sim and kits/python
    (cd core && cargo test -- --ignored live_)     # live CLI tests: they call real Claude Code and Codex and cost money
    (cd ui && npx vitest run && npm run build)

Tests never call a real model unless they are `live_`. Never start arenad while tests run: the tests make
databases that arenad's startup cleanup would drop.

## Limits and not built yet

- Only `postgres` has been used in an arenad run with agents; every other integration is checked on its own.
- Only one benchmark is included, the retail example. arenad runs one benchmark at a time.
- Teams are a fixed sequence of agents with handoff notes; only the last agent writes.
- API mode has not been measured (no key was set).
- Isaac Sim on hardware: the `isaac` backend of `isaac-sim` has never run on Isaac Sim 6.2 (needs a 16 GB GPU).
- Cross-part calls: protocol v1 has no calls between parts. `proposal_id` is accepted by the `ridgeline-hub`
  and `isaac-sim` tools and not checked.
- A world with agents made of several integrations (for example ERPNext, the hub, Sculptured and the plant
  together) has not been built or run.
- Comparing a world made only of replay parts: there is no final save to compare, so compare answers 409 and
  the branch fails with "no part of this world can be checked against the task's expected result".
- Re-rooting the kept base and gold saves of `mariadb` and `erpnext`: those saves pin the binary log, so a
  long-lived server's binlog grows until they are re-rooted. Not built; no benchmark uses these integrations yet.
- Community publishing of integrations and benchmarks (design only). Packages install from a local folder.
- Process checkpointing (CRIU), fault injection, fault search and design search.
- SGLang or vLLM hosting, serving-state capture and chain-of-thought capture.
- Dashboards.

## Attribution

The retail data in `data/retail/` (`db.json`, `tasks.json`, `policy.md`) comes from [tau2-bench](https://github.com/sierra-research/tau2-bench), MIT License, Copyright (c) 2025 Sierra Research.
