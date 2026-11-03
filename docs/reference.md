# Reference

arenad's HTTP API, its settings, what uses disk, and how to run the tests. For the ideas behind them see
[concepts.md](concepts.md).

## HTTP API

All JSON on `http://127.0.0.1:4510`, except the event stream. Errors are `{error}` with a status code.

| Route | What it does |
|---|---|
| `GET /api/health` | `{open_available, providers, claude, codex, benchmark, disk: {bytes, limit_bytes}}`. `providers` is each model provider as the configuration alone tells it, in the shape of `/api/providers` without `models`; a local server is not asked, so it is `available: false, checked: false` with why `not checked (GET /api/providers asks the server)`. `open_available` is true when some provider can take calls, judged from the configuration (a configured local server counts, up or not) |
| `GET /api/providers` | `[{id, kind, label, available, checked, why?, models?}]`, one per model provider; see [Model providers](#model-providers) |
| `GET /api/tasks` | the benchmark's tasks, `[{id, reason}]` |
| `GET /api/team/default` | the default team |
| `GET /api/runs` | the runs with their branches |
| `POST /api/runs` | `{task_id, team}`: start a run; returns `{run_id, branch_id}`. Each agent is `{id, name, kind, model, prompt, provider?}`; 400 when an API-mode agent's provider is unknown or unavailable, or has no default model and the agent names none |
| `DELETE /api/runs/{id}` | delete a run and what it saved (409 while a branch runs) |
| `GET /api/branches/{id}` | a branch with its events |
| `GET /api/branches/{id}/stream` | Server-Sent Events: each `event`, then the final `status` |
| `GET /api/branches/{id}/rows?seq=N&keys=<kind>/<id>,...[&part=]` | the state of those keys just after step N (at most 50 keys) |
| `POST /api/branches/{id}/fork` | `{at_seq, changes: {<agent id>: {prompt?, model?, kind?, provider?}}, restore: "exact"\|"approximate"}` (default exact); returns `{branch_id, start_seq, moved_from, reason}`. The changes are checked on the team the fork would run, as `POST /api/runs` checks a team |
| `GET /api/branches/{id}/restore-plan?at_seq=N&restore=exact\|approximate` | `{parts: [{part, how, estimate_seconds?}]}`, or 409 with the refusal |
| `GET /api/compare?a=<branch>&b=<branch>` | `{a, b, first_divergent_seq, differing_rows, not_compared}` |
| `GET /api/world` | the benchmark's parts with what each integration reports (version, restore, clock use, tool names) and whether it answered (each gets 5 s; a silent one shows `did not answer within 5 s`) |
| `GET /api/usage` | `{integrations: [{integration, parts, bytes, error?}], total_bytes, limit_bytes}`: the bytes each integration keeps (its own `usage` plus its save folders) and the world parts that use it. An integration whose `usage` fails or doesn't answer within 5 s has `bytes: 0` and an `error`, and adds nothing to `total_bytes` |
| `POST /api/gc` | runs the startup cleanup and returns `{removed, freed_bytes, errors}`; it keeps every save a recorded run names and the shared base and gold saves; 409 while agents are running, and new runs and forks answer 409 while it runs. It has no deadline: it answers when every integration's `gc` has (an integration that never answers keeps the cleanup on until it exits), and a client that stops waiting does not end it |

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
  `integrations:benchmarks` in the repository root).
- `ARENA_BIN_DIR`: what `${ARENA_BIN_DIR}` in a manifest stands for (default `core/target/release`).
- `ARENA_SAVES`: where saves are kept (default `core/saves`, ignored by git). Integrations that run in Docker or
  on their own servers also keep data elsewhere.
- `ARENA_MAX_DB_BYTES`: disk limit, see [Disk](#disk) (default 5 GiB, `5368709120`).
- `ARENA_CLOCK_SPEED`: speed of the world clock (default 1, real time; 0 stops it; anything that isn't a number
  of at least 0 counts as 1). Unlike the others, it is read each time a run or fork starts a branch, not once at
  startup.
- `OPENROUTER_API_KEY`, `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`: the keys of the OpenRouter, OpenAI and Anthropic
  providers of API mode. Each key turns its provider on. Pick the provider and model per agent on the Team page
  (or per fork in "Change and rerun").
- `ARENA_LOCAL_MODELS`: OpenAI-compatible servers to use as providers, `name=url[,name=url…]`, each the provider
  `local:<name>`. The name matches `^[a-z0-9_-]+$` and the URL is `http` or `https`. A bad entry, or a name
  given twice after its first, is skipped with a warning on stderr that never shows the URL.
- `ARENA_LOCAL_KEY_<NAME>`: an optional key for `local:<name>`, sent as a bearer token (`<NAME>` is the name
  upper-cased, with `-` as `_`).
- `ARENA_ANTHROPIC_MAX_TOKENS`: the `max_tokens` of a Claude call whose request names none (default 16000). A
  value that isn't a positive whole number is reported at startup and the default is used.
- `ARENA_UI_DIST`: a built UI folder for arenad to serve at `/`.

A blank key counts as unset. A key a header can't carry is reported at startup (by its variable's name) and
ignored. arenad never logs, records or returns a key. The CLI agents don't get them: each CLI keeps only its own
vendor's key (see [concepts.md](concepts.md#teams-and-agents)).

Default paths are relative to the repository root, fixed when arenad is built, so they hold wherever arenad is started
from. The retail example reads its data from `data/retail` (relative to its package folder, `../../data/retail`);
`ARENA_RETAIL_DATA` overrides that when you run its program directly, but arenad does not pass it on.

## Model providers

An API-mode agent (`kind: "open"`) names its provider in `provider`; without one it uses `openrouter`, so teams
and runs made before providers keep their meaning. The agent loop speaks OpenAI chat completions to arenad's
gateway, and the gateway talks to the provider:

| Provider | Endpoint | Key | Models listed by `/api/providers` | Default model |
|---|---|---|---|---|
| `openrouter` | `https://openrouter.ai/api/v1/chat/completions` | `OPENROUTER_API_KEY`, bearer | not asked (type an OpenRouter model id) | `deepseek/deepseek-chat-v3.1` |
| `openai` | `https://api.openai.com/v1/chat/completions` | `OPENAI_API_KEY`, bearer | `GET /v1/models`, without embedding, audio, image, moderation and other non-chat models | none: name one |
| `anthropic` | `https://api.anthropic.com/v1/messages` (the Messages API, translated) | `ANTHROPIC_API_KEY`, `x-api-key` | `GET /v1/models` | none: name one |
| `local:<name>` | `<url>/chat/completions` of the `ARENA_LOCAL_MODELS` entry | `ARENA_LOCAL_KEY_<NAME>`, bearer, if set | `GET <url>/models` | none: name one |

A local provider is any server that speaks the OpenAI chat-completions API, such as Ollama, vLLM, LM Studio or
llama.cpp's server. For example:

```bash
cd core && ARENA_LOCAL_MODELS="ollama=http://127.0.0.1:11434/v1,vllm=http://127.0.0.1:8000/v1" cargo run -p arenad
```

Then pick `local:ollama` or `local:vllm` as an agent's provider and name a model the server serves (for Ollama, a
model you pulled, such as `llama3.3`).

**`GET /api/providers`** asks each local server, and OpenAI and Anthropic when their key is set, for its models,
all at once, each within 3 s; a list over 1 MB is given up. `available` is false, with a plain `why`, when a key
is not set (`OPENAI_API_KEY is not set`), when a list was refused for its key (`the key was rejected`), or when a
local server doesn't answer (`<host:port> did not answer within 3 s`). When OpenAI's or Anthropic's list fails
otherwise, the provider stays available with the reason, and the model is typed in. `checked` is true when the
provider itself was asked. `why` names a host and port, never a URL's credentials. The web UI shows one entry per
provider under the top bar (glyph, state and reason), and its Check button asks this route.

What the gateway does per provider:

- **OpenRouter:** the request goes out as the agent sent it, plus `usage: {include: true}` so OpenRouter
  reports the cost. Only OpenRouter gets that field.
- **OpenAI and local servers:** the request goes out as the agent sent it.
- **Anthropic:** the request is translated to the Messages API (`anthropic-version: 2026-10-21`): system
  messages become `system`, tools get `input_schema`, tool calls and tool results become `tool_use` and
  `tool_result` blocks, and `max_tokens` is the request's own or `ARENA_ANTHROPIC_MAX_TOKENS`. Nothing else is
  carried over, so sampling fields such as `temperature` are dropped, and a forced tool choice becomes `auto`. The
  answer is translated back: `stop_reason` `end_turn` is `stop`, `tool_use` is `tool_calls`, `max_tokens` is
  `length` and `refusal` is `content_filter`; usage becomes prompt, completion and total tokens.
- Every call is recorded in the canonical (OpenAI) form, with `provider` (the id that answered) and, when the
  provider's own answer differs from the canonical one (Anthropic), `provider_response`. A 401 or 403 answer is
  recorded without its error message, which may quote the key.
- A provider's error status is passed on to the agent loop; no answer is a 502. The loop tries a call that got
  429, 503 or 529 again, up to 3 times, after the provider's `retry-after` or else 1 s, doubling each time, at
  most 20 s; every try is recorded. A call that still fails, or fails another way, ends the agent with the
  provider's message.
- A model that refuses (`content_filter`, Claude's `refusal`) ends the agent as failed: `the model refused`,
  with the category when Claude gives one.

Cost per provider: OpenRouter's is the cost it reports (`cost_source: "openrouter"`). OpenAI and Anthropic
report no price and arenad keeps no price table, so their tokens are recorded and the cost is unknown
(`cost_usd: null`); the UI shows `unknown`. A local server's cost is 0 with
`cost_source: "local (no API charge)"`; the UI shows `no API charge`.

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
  `POST /api/gc` runs the same cleanup. Neither has a deadline: an integration's `gc` may stop instances, so
  new runs and forks wait (409) until every integration has answered, however long that takes. Start arenad
  only when no test is running, since tests make unreferenced saves.
- The tests drop what they create, even when an assertion fails. `core/scripts/leakcheck.sh` (run from `core`)
  runs each of the core's test binaries and fails if one leaves databases behind: Postgres `pi_`, `ps_`,
  `branch_`, `snap_` and `arena_test` databases, and MariaDB `mi_` and `mx_` ones when `LEAKCHECK_MARIADB` names
  the server container.

## Tests

```bash
(cd core && cargo test)                        # needs the Postgres of deploy/compose.yml
cargo test                                     # at the repository root: kit, integrations, benchmarks (some need the Postgres)
cargo test -- --ignored live_                  # live integration tests: they start Docker containers or real servers
(cd integrations/ridgeline-hub && uv run --with pytest pytest -q)    # likewise integrations/isaac-sim and kits/python
(cd core && cargo test -- --ignored live_)     # live CLI tests: they call real Claude Code and Codex and cost money
(cd ui && npx vitest run && npm run build)
```

The tests of `integrations/ridgeline-hub` and `integrations/isaac-sim` read the Ridgeline factory example from
`Sculptured-Examples/factory` next to the arena checkout (their READMEs say which folders). **That example is not
public yet**: without it every test of those two packages is skipped, and the packages can't start. The Sculptured
program that `integrations/sculptured` runs is not published yet either; its live test is skipped without it.

Tests never call a real model unless they are `live_`. Never start arenad while tests run: the tests make
databases that arenad's startup cleanup would drop.

CI (`.github/workflows/ci.yml`) runs the format, Clippy and test checks of both Cargo workspaces, the Python
kit's tests, an import check of the two Python integrations, and the UI's tests and build, but no `live_`
test. See [CONTRIBUTING.md](../CONTRIBUTING.md) for the full list of
checks before a pull request.
