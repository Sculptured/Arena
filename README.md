# Reversible arena

The arena runs a team of AI agents on a customer-service task from tau2-bench retail, with the retail data in Postgres. It records every step together with a snapshot of the database. You can go back to any step, change the prompt, model or harness of an agent, and rerun from there with the database as it was at that step. Then you compare the two runs.

## The demo in one screen

1. Start a run: pick a task and a team (default: triage, then resolver).
2. Click a step to see its arguments, its result, and the rows it read or wrote, as they were at that step.
3. Fork at a step with a change (for example a new resolver prompt, or the resolver run by Codex). The database is restored to just before that step, and the step is redone.
4. Compare the two branches: outcome, time, tokens, cost, the first step where they differ, and the database rows that differ.

## Quick start

Needs Docker, Rust, Node, and the `claude` and/or `codex` CLIs on your PATH for CLI mode.

    docker compose -f deploy/compose.yml up -d     # Postgres, published on loopback only (127.0.0.1:5544)
    cd core && cargo run -p arenad                 # API on http://127.0.0.1:4510
    cd ui && npm install && npm run dev            # UI on http://127.0.0.1:4520

Or build the UI and let arenad serve it:

    cd ui && npm install && npm run build
    cd ../core && ARENA_UI_DIST=../ui/dist cargo run -p arenad    # UI and API on :4510

`GET /api/health` shows which CLIs were found and whether open mode is available.

## The two modes

- **CLI mode** (Claude Code or Codex): the arena starts the CLI headless and gives it only the retail tools through an MCP endpoint of arenad. It records every tool call (arguments, result, rows read and written), the handoff note, and the tokens and cost the CLI reports. It does not see the raw model calls.
- **Open mode**: arenad runs the agent loop and sends each model call through its gateway to OpenRouter. It records everything CLI mode does, plus every model call. It needs `OPENROUTER_API_KEY`. No open-mode run has been measured yet (no key was set for the measurements below).

Isolation of the CLI agents (from the Task 8 live checks):
- Claude Code: settings sources empty, strict MCP config, slash commands off, built-in tools off (`--tools ""`), so the only tools are the retail MCP tools.
- Codex: user config, rules and session storage ignored; apps, plugins, hooks and similar features disabled; web search off; read-only sandbox. Codex's built-in shell cannot be turned off. A probe from inside the sandbox could not open a socket, not even to the loopback Postgres port (`Operation not permitted`). Codex still lists its sub-agent tools (`collaboration.*`), which run under the same restrictions.
- Both CLIs run with `PG*`, `ARENA_PG_URL`, `DATABASE_URL` and `OPENROUTER_API_KEY` removed from their environment, in a fresh temporary working directory.
- A CLI session cannot be resumed in the middle, which drives the fork rules below.

## Forking

You choose a step N and a change (prompt, model, or kind of one agent).
- Step N is redone with the change. The database is copied from the last snapshot taken before step N.
- Agents that finished before step N keep their recorded output (for example the handoff note).
- The fork point moves back to the start of an agent when: the agent active at step N is a CLI agent (a CLI session can't be cut at a tool call); the change switches its mode; or the change is for an agent that finished before step N (then it moves to the start of the earliest changed agent). The response and the UI say where the fork started (`start_seq`), where you asked for (`moved_from`), and why (`reason`).
- A change is never silently ignored.
- If there is no restore point for the step, the fork is refused with an error.
- Changing `kind` resets the agent's model to the new mode's default, unless the same change also sets a model. An empty model in a change clears the model.

## Cost labels

- `openrouter`: the cost OpenRouter reports (open mode).
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
| Open mode | not run | | | no `OPENROUTER_API_KEY` set; `open_available` was false |

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
- `OPENROUTER_API_KEY`: turns on open mode.
- `ARENA_UI_DIST`: a built UI folder for arenad to serve.
- `ARENA_MAX_DB_BYTES`: disk limit, see below (default 5 GiB).

## Disk

What uses space: each branch, snapshot and gold database is a full copy of the retail database (about 21 MB), and every successful write makes a snapshot. Runs and forks add up; the arena Postgres keeps them until you delete the run. The Rust build directory `core/target` also grows; `cd core && cargo clean` frees it.

- The top bar shows `DB used / limit`. When the `branch_`, `snap_` and `gold_` databases together reach the limit, new runs and forks are refused (HTTP 507) until you delete old runs. The limit is `ARENA_MAX_DB_BYTES`, read once at startup (default 5 GiB, `5368709120`).
- Delete a run from the Runs page (Delete run, then confirm), or `curl -X DELETE http://127.0.0.1:4510/api/runs/<run_id>`. This drops all its branch and snapshot databases and its recorded events. A run with a running branch can't be deleted.
- At startup arenad drops `branch_` and `snap_` databases that no recorded branch or event refers to (left by crashes), and leftover `gold_tmp_` databases, and logs `gc: dropped N databases (X MB)`. It never touches `world_base`, `gold_*`, `arena` or other databases. Start arenad only when no test is running, since tests make unreferenced databases.
- The tests drop every database they create, even when an assertion fails. `core/scripts/leakcheck.sh` runs each test binary and fails if one leaves databases behind.

## Tests

    cd core && cargo test                          # needs the Postgres above
    cd core && cargo test -- --ignored live_       # live CLI tests: they call real Claude Code and Codex and cost money
    cd ui && npx vitest run && npm run build

Never start arenad while tests run: the tests make databases that arenad's startup cleanup would drop.

## Not built yet

- Fault injection.
- SGLang or vLLM hosting, serving-state capture and chain-of-thought capture.
- Dashboards.
- Benchmarks other than tau2-bench retail.
- Fault search and design search.
- Integration with Sculptured.
- Community packages.

## Attribution

The retail data in `data/retail/` (`db.json`, `tasks.json`, `policy.md`) comes from [tau2-bench](https://github.com/sierra-research/tau2-bench), MIT License, Copyright (c) 2025 Sierra Research.
