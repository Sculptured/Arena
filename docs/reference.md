# Reference

arenad's HTTP API, its settings, what uses disk, and how to run the tests. For the ideas behind them see
[concepts.md](concepts.md).

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
- `ARENA_CLOCK_SPEED`: speed of the world clock (default 1, real time).
- `OPENROUTER_API_KEY`: turns on API mode. Pick the model per agent on the Team page (or per fork in "Change and
  rerun") by typing an OpenRouter model id.
- `ARENA_UI_DIST`: a built UI folder for arenad to serve at `/`.

Default paths are relative to the repository root, fixed when arenad is built, so they hold wherever arenad is started
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
