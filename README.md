# Reversible arena

The arena runs a team of AI agents on a task, in a world made of integrations. Today the one benchmark is customer service from tau2-bench retail, and its world is one Postgres database. The arena records every step and saves the world at that step. You can go back to any step, change the prompt, model or harness of an agent, and rerun from there with the world as it was at that step. Then you compare the two runs.

Docs: `docs/protocol.md` (the protocol between arenad and an integration), `integrations/README.md` (how to write an integration), `docs/specs/2026-10-31-integrations-design.md` (the design).

## The demo in one screen

1. Start a run: pick a task and a team (default: triage, then resolver).
2. Click a step to see its arguments, its result, and the rows it read or wrote, as they were at that step.
3. Fork at a step with a change (for example a new resolver prompt, or the resolver run by Codex). The database is restored to just before that step, and the step is redone.
4. Compare the two branches: outcome, time, tokens, cost, the first step where they differ, and the database rows that differ.

## Quick start

Needs Docker, Rust, Node, and the `claude` and/or `codex` CLIs on your PATH for CLI mode.

    docker compose -f deploy/compose.yml up -d     # Postgres, published on loopback only (127.0.0.1:5544)
    cargo build --release                          # in this folder: the postgres integration and the retail benchmark
    cd core && cargo run -p arenad                 # API on http://127.0.0.1:4510

arenad starts the integration programs itself, from `core/target/release` (`ARENA_BIN_DIR`). It refuses to start if
the benchmark program is not built.
    cd ui && npm install && npm run dev            # UI on http://127.0.0.1:4520

Or build the UI and let arenad serve it:

    cd ui && npm install && npm run build
    cd ../core && ARENA_UI_DIST=../ui/dist cargo run -p arenad    # UI and API on :4510

`GET /api/health` shows which CLIs were found and whether OpenRouter mode is available.

Control routes: `GET /api/world` lists the benchmark's parts with what each integration reports (version, restore, clock use, tool names) and whether it answered (each gets 5 s; a silent one shows `did not answer within 5 s`); `GET /api/usage` gives `{integrations: [{integration, parts, bytes}], total_bytes, limit_bytes}`, the bytes each integration keeps and the world parts that use it; `POST /api/gc` runs the startup cleanup and returns `{removed, freed_bytes, errors}` (it keeps every save a recorded run names; it answers 409 while agents are running, and new runs and forks answer 409 while it runs).

## The two modes

- **CLI mode** (Claude Code or Codex): the arena starts the CLI headless and gives it only the retail tools through an MCP endpoint of arenad. It records every tool call (arguments, result, rows read and written), the handoff note, and the tokens and cost the CLI reports. It does not see the raw model calls.
- **OpenRouter mode** (`kind: "open"` in the API): arenad runs the agent loop and sends each model call through its gateway to OpenRouter. It records everything CLI mode does, plus every model call. It needs `OPENROUTER_API_KEY`. No OpenRouter run has been measured yet (no key was set for the measurements below).

Isolation of the CLI agents (from the Task 8 live checks):
- Claude Code: settings sources empty, strict MCP config, slash commands off, built-in tools off (`--tools ""`), so the only tools are the retail MCP tools.
- Codex: user config, rules and session storage ignored; apps, plugins, hooks and similar features disabled; web search off; read-only sandbox. Codex's built-in shell cannot be turned off. A probe from inside the sandbox could not open a socket, not even to the loopback Postgres port (`Operation not permitted`). Codex still lists its sub-agent tools (`collaboration.*`), which run under the same restrictions.
- Both CLIs run with `PG*`, `ARENA_PG_URL`, `DATABASE_URL` and `OPENROUTER_API_KEY` removed from their environment, in a fresh temporary working directory.
- A CLI session cannot be resumed in the middle, which drives the fork rules below.

## The world and its integrations

A benchmark names its world: a list of parts, each an instance of an integration. An integration is an isolated program, like a device driver, that speaks `arena-part/1` with arenad (`docs/protocol.md`). It starts instances, runs tool calls, saves its state, restores it, shows it and compares it. The core has no code for any particular system. The retail world is one part, `retail`, an instance of `postgres`.

arenad saves every part at every step (a tool call that wrote, plus any part with a clock that changed) and records the saves with the step. Parts with `uses_clock` get the arena clock, at real time while agents work. The clock pauses while saving.

### Restore modes

A fork asks for one restore mode for the whole world. Each integration declares what it can do (`describe`):

- **`exact_restore`**: the part is exactly as it was. From an exact save when the part has one (fast), or by replaying from the start with the same seed and every recorded input up to the step (slower the later the step). If a replay gives a different result than the recorded one, the fork fails with "replay of <part> diverged at step N, so it can't be restored exactly". A replay is refused when the part's fingerprint (versions, GPU, source hashes) changed.
- **`approximate_restore`**: from a save that may not capture everything (fast). Branches restored this way are labelled "approximate restore" in the run page, the branch list and compare.

| Part declares | exact | approximate |
|---|---|---|
| exact save | from its save | same |
| replay and approximate | replay | from its save, approximately |
| replay only | replay | replay |
| approximate only | refused, naming the part | from its save, approximately |
| none | refused | refused |

`GET /api/branches/{id}/restore-plan?restore=exact|approximate` and the rerun panel show, per part, how it will be restored and, for a replay, an estimate of the time.

### What is built

Columns: **exact** is `restore.exact` (`save`, `replay` or `none`), **approximate** is `restore.approximate`, **clock** is `uses_clock`, all read from each program's `describe`. **Verified** says what ran, and on what. A "live" test is `#[ignore]`d with the prefix `live_` and needs Docker or a real server. Each folder's README has the details.

| Name | What it is | Exact | Approximate | Clock | Verified |
|---|---|---|---|---|---|
| `postgres` | A Postgres database per instance | save | no | no | Tests against the `postgres:17-alpine` server of `deploy/compose.yml`, with the `arena-part-check` run (`integrations/postgres/tests/part.rs`); the retail world runs on it |
| `folder` | A folder of files per instance | save | no | no | Tests on local disk with the `arena-part-check` run (`integrations/folder/tests/part.rs`); no service involved |
| `service` | Any Docker Compose project per instance | save | no | no | `live_service_roundtrip` against a real Redis 6.2 project in Docker. No `arena-part-check` run |
| `mariadb` | A MariaDB database per instance (saves are binary-log positions) | save | no | no | Seven `live_mariadb_*` tests against MariaDB 11.8 in Docker, including `arena-part-check` |
| `erpnext` | An ERPNext site per instance (saves are binary-log positions plus the site's files) | save | no | no | `live_erpnext_roundtrip` against ERPNext v16.53.0 in Docker, including `arena-part-check` |
| `sculptured` | A Sculptured server per instance, for approvals | save | no | no | `live_sculptured_roundtrip` against a real `sculptured` build; `arena-part-check` passed once by hand |
| `ridgeline-hub` | The factory's suppliers, carriers, insurer and customers | save | no | yes | Python tests against the real hub `market.py`, with `arena-part-check`. No services, no arenad run |
| `isaac-sim` | The factory plant, Isaac Sim physics or a stand-in | replay | yes | yes | **Stand-in only.** Python tests with `arena-part-check`. The `isaac` backend has never run on Isaac Sim |
| `isaac-sim-mirror` | A read-only view of a running plant | none | no | yes | A stub HTTP server only. A world containing it can't be rerun |
| `counter` (Rust), `counter-py` (Python) | Examples in the kits: one integer per instance | save (flavours: replay, approximate) | no (flavour) | yes | Kit and core tests (`kits/*/tests`, `core/crates/parts/tests`, arenad tests) |

The retail benchmark (`benchmarks/retail`) is not an integration. It brings the tasks, the tools, the data loader and the gold actions, and names a world of one `postgres` part. It runs through arenad in the tests. The live Claude Code and Codex runs under "Measurements" were made before the move to integrations and have not been repeated.

Only `postgres` (through the retail benchmark) has been used in an arenad run with agents. The other integrations are checked on their own, not inside a team's run.

### Write an integration

A folder with an `arena-package.toml` (`kind`, `name`, `version`, `protocol`, `run`, `needs`, `config_schema`, and `[[example]]` tool calls) and a program that speaks the protocol. The Rust kit (`kits/rust`, crate `arena-part`) and the Python kit (`kits/python`, package `arena_part`) read the lines and check the ids; you fill in `describe`, `start`, `call_tool`, `save`, `inspect`, `diff` and a few more. `integrations/README.md` walks through the counter example. Then check it:

    cargo build                                    # debug build of the checker and your integration
    ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/<name> [--config file.json]

`arena-part-check` runs the program and checks describe, start, the examples, save and restore, diff, replay and `drop_save`, prints a report and exits non-zero on a failure. Install a package by putting its folder under one of the `ARENA_PACKAGES` roots. A `command` run is for programs you trust. An `image` run is boxed in a container: no network unless the manifest asks for host networking (`needs.network = "host"`), no environment from arenad, and only its own saves folder, mounted at `/saves`.

## Forking

You choose a step N and a change (prompt, model, or kind of one agent).
- Step N is redone with the change. Each part of the world is put back as it was just before step N, in the restore mode you choose (see "Restore modes").
- Agents that finished before step N keep their recorded output (for example the handoff note).
- The fork point moves back to the start of an agent when: the agent active at step N is a CLI agent (a CLI session can't be cut at a tool call); the change switches its mode; or the change is for an agent that finished before step N (then it moves to the start of the earliest changed agent). The response and the UI say where the fork started (`start_seq`), where you asked for (`moved_from`), and why (`reason`).
- A change is never silently ignored.
- If there is no restore point for the step, or a part can't be restored in the mode asked for, the fork is refused with an error that names the part.
- Changing `kind` resets the agent's model to the new mode's default, unless the same change also sets a model. An empty model in a change clears the model.

## Cost labels

- `openrouter`: the cost OpenRouter reports (OpenRouter mode).
- `api_equivalent_estimate`: Claude Code's own `total_cost_usd`. It is what the same tokens would cost at API prices, not what a subscription pays.
- No label and no dollar figure: Codex reports tokens only.
- Dollars are shown only when every agent in the branch has a dollar figure; otherwise only tokens are shown. Tokens are always summed. When agents have different sources the label is `mixed`.

## Measurements

Snapshot copy time: median 152 ms over 10 snapshots in the Task 4 test; three runs gave 152, 152 and 221 ms. Each snapshot is a full copy of the 21 MB retail database. The machine was the development laptop.

Live runs on task 27 (2026-10-30), through the HTTP API, one run each. "A" is the default team (Claude Code to Claude Code, resolver prompt unchanged, model `sonnet`). Seconds, tokens and cost are the compare output. For run A and run B they cover the whole branch (triage and resolver). For a fork they cover only the steps the fork redid (here the resolver), because the steps before the fork point are copied, not rerun.

| Run | Status | Seconds | Tokens | Cost |
|---|---|---|---|---|
| A: default team (Task 13) | passed | 23.574 | 104427 | $0.0996256 (api_equivalent_estimate) |
| A fork: resolver prompt changed, forked at step 12 (Task 13) | passed | 8.911 | 36036 | $0.0296177 (api_equivalent_estimate) |
| B: triage Claude Code, resolver Codex (new run, model unset) | passed | 50.708 | 166583 | $0.0407897 (api_equivalent_estimate, triage only, from its agent_end; Codex has no dollar figure, so the compare output shows no dollars for B) |
| C1: fork of A at step 12, resolver kind changed to `codex`, model left as `sonnet` | infra_error | 1.725 | 0 | none |
| C2: same fork, resolver kind `codex` and model `gpt-6.1-sol` | passed | 29.159 | 106955 | none (Codex reports tokens only) |
| OpenRouter mode | not run | | | no `OPENROUTER_API_KEY` set; `open_available` was false |

Notes:
- The two fork rows (and C1/C2) cover the resolver only. For comparison, A's resolver alone was 8.796 s, 45932 tokens, $0.0508491 (from the agent_end event).
- B and C2 passed the database check (no differing rows against the gold result). The Codex resolver was slower than the Claude resolver (34.6 s in B and 28.8 s in C2, against 8.8 s in A; Claude's 8.9 s fork with a longer prompt was similar) and used more tokens (107721 in B and 106955 in C2, against 45932). Codex token counts include cached input, so they are not comparable one to one with Claude's. One run each, so this is an observation, not a benchmark.
- C1 was measured before the fix that makes a kind-only fork reset the model (a kind-only fork now resets the model to the new mode's default, so C1 would not fail this way). It failed because Codex with a ChatGPT account does not support the model `sonnet`: `The 'sonnet' model is not supported when using Codex with a ChatGPT account.` It is not a usage limit. The fork response was `{"start_seq":12,"moved_from":null,"reason":null}`, and the branch ended `infra_error`. C2 is the one retry, with the model set to the one in my Codex config.
- Compare, A against C2: `first_divergent_seq` 13, `differing_rows` empty. A against B: `first_divergent_seq` 13, `differing_rows` empty. A against the Task 13 prompt fork: `first_divergent_seq` 16, `differing_rows` empty. A against C1: `first_divergent_seq` 13, one differing row, `orders/#W7181492` (A made the exchange, the failed branch did not).
- None of the forks moved (`moved_from` and `reason` were `null`): step 12 is already the start of the resolver.

## Configuration

Environment variables, read once at startup:
- `ARENA_PG_URL`: Postgres connection (default `postgres://arena:arena@127.0.0.1:5544/postgres`).
- `ARENA_DATA`: folder with the retail data (default `../data/retail`, relative to `core`).
- `ARENA_PORT`: HTTP port (default 4510).
- `OPENROUTER_API_KEY`: turns on OpenRouter mode. Pick the model per agent on the Team page (or per fork in "Change and rerun") by typing an OpenRouter model id such as `deepseek/deepseek-chat-v3.1`, the default when the field is empty.
- `ARENA_UI_DIST`: a built UI folder for arenad to serve.
- `ARENA_MAX_DB_BYTES`: disk limit, see below (default 5 GiB).
- `ARENA_PACKAGES`: `:`-separated folders whose subfolders hold an `arena-package.toml` (default `integrations:benchmarks`, next to `core`).
- `ARENA_BIN_DIR`: where `${ARENA_BIN_DIR}` in a manifest points (default `core/target/release`).
- `ARENA_SAVES`: where saves are kept (default `core/saves`, ignored by git). Integrations that run in Docker or on their own servers keep their data elsewhere too.
- `ARENA_BENCHMARK`: the benchmark new runs use (default `retail`).
- `ARENA_CLOCK_SPEED`: speed of the world clock (default 1, real time).

## Disk

What uses space: every save of every part, and the databases or files a part keeps outside `ARENA_SAVES`. For the retail world, every successful write makes a save, a full copy of the retail database (about 21 MB); other integrations save less or more (see their READMEs). Runs and forks add up until you delete the run. The Rust build directory `core/target` also grows; `cd core && cargo clean` frees it.

- The top bar shows `Saves used / limit`. It counts each integration's `usage` plus the size of `ARENA_SAVES`. When it reaches the limit, new runs and forks are refused (HTTP 507) until you delete old runs. The limit is `ARENA_MAX_DB_BYTES`, read once at startup (default 5 GiB, `5368709120`).
- Delete a run from the Runs page (Delete run, then confirm), or `curl -X DELETE http://127.0.0.1:4510/api/runs/<run_id>`. This drops its saves, stops its live instances and removes its recorded events. The shared base and gold saves stay. A run with a running branch can't be deleted.
- At startup arenad asks each integration of the benchmark's world to drop saves that no recorded event names (left by crashes), and logs `gc: dropped N saves (X MB)`. An integration without `gc` is skipped. Start arenad only when no test is running, since tests make unreferenced saves.
- The tests drop what they create, even when an assertion fails. `core/scripts/leakcheck.sh` runs each test binary and fails if one leaves databases behind (it counts `branch_`, `snap_` and test metadata databases).

## Tests

    cd core && cargo test                          # needs the Postgres above
    cargo test                                     # in this folder: kit, integrations, benchmark (the Postgres above for some)
    cargo test -- --ignored live_                  # live integration tests: they start Docker containers or real servers
    cd integrations/ridgeline-hub && uv run --with pytest pytest -q    # likewise isaac-sim, and kits/python
    cd core && cargo test -- --ignored live_       # live CLI tests: they call real Claude Code and Codex and cost money
    cd ui && npx vitest run && npm run build

Never start arenad while tests run: the tests make databases that arenad's startup cleanup would drop.

## Not built yet

- Community publishing of integrations (design only). Packages install from a local folder.
- Cross-part calls: a hub shipment reaching Isaac Sim, and approvals gating other parts' tools. `proposal_id` is accepted by the hub and Isaac Sim tools and not checked. Protocol v1 has no calls between parts.
- Isaac Sim on hardware. The `isaac` backend has never run on Isaac Sim 6.2 (needs a 16 GB GPU; the DGX Spark is the target).
- A factory world and benchmark with agents (ERPNext, hub, Sculptured and the plant together), and any test of an integration inside a team's run other than `postgres`.
- The design's `ridgeline-plant` integration. The plant is `isaac-sim` (stand-in or Isaac Sim) instead.
- Comparing a world made only of replay parts: there is no final save to compare, so compare answers 409 and the branch fails with "no part of this world can be checked".
- Re-rooting the kept base and gold saves of `mariadb` and `erpnext`: those saves pin the binlog, so a long-lived server's binlog grows until they are re-rooted. Not built, and no benchmark uses these integrations yet.
- Process checkpointing (CRIU).
- Fault injection.
- SGLang or vLLM hosting, serving-state capture and chain-of-thought capture.
- Dashboards.
- Benchmarks other than tau2-bench retail.
- Fault search and design search.
- OpenRouter mode has not been measured (no key).

## Attribution

The retail data in `data/retail/` (`db.json`, `tasks.json`, `policy.md`) comes from [tau2-bench](https://github.com/sierra-research/tau2-bench), MIT License, Copyright (c) 2025 Sierra Research.
