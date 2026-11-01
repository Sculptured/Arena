# Integrations: any part of a system, saved and restored (design)

> **As built (2026-11-01).** This is the design as written before the build; the build differs in two
> places. Task 13's `ridgeline-plant` became `ridgeline-hub` (the factory's outside companies on the arena
> clock: `integrations/ridgeline-hub/`), and Task 15 wraps the factory's own Isaac plant as `isaac-sim`
> (Isaac Sim or a stand-in, with `isaac-sim-mirror` for a running plant) instead of a generic scene. The
> integrations table in the arena `README.md` describes what was built.

Status: 2026-10-31. The user asked for this and said "just build the set for it to work"; they then
answered the open questions (recorded under "Decided with the user"). Choices marked **(my call)** were
not reviewed by them.

## Decided with the user (2026-10-31)

- Integrations hold state; **benchmarks are separate packages** that bring tasks, tools and a check. The
  retail benchmark is its own package; `postgres` is a generic integration.
- ERPNext (a wrapper on a database) saves **only the changes**: a save is a position in MariaDB's binary
  log, and a restore applies the log up to it, like a database's point-in-time recovery. No stop and start.
- The world runs on the **arena clock**, at real time while agents work, because agents act in real time.
- Replay divergence fails the rerun. It should not happen for seeded, deterministic parts; it is a safety
  net against hidden nondeterminism (GPU physics, real clocks, thread timing, a different machine).
- Isaac Sim gets its own agent tools, plus a mirror-only setting.
- `../sculptured-factory` stays untouched.
- UI: user doesn't mind; both UIs get the restore choice and labels **(my call)**.

## Intent

Today the arena's world is one Postgres database, and the arena core knows it intimately: `world` crate,
`CREATE DATABASE … TEMPLATE` snapshots, retail tools. The user wants any piece of a system to be a part of
the world whose state is saved at every step and restored on a rerun: an ERPNext, a plant simulator, a
Sculptured server, an Isaac Sim scene. The model is device drivers in the Linux kernel:

- The core supplies **primitives**. It never contains code for a particular system, Postgres included.
- Each **integration** lives in its own isolated folder under `arena/integrations/`, uses only the
  primitives, and is written in code by anyone. Later they are published through Community.

Example target, not built as a test in this round: the Ridgeline factory (`../sculptured-factory`):
ERPNext, the plant simulator and market, Sculptured approvals, Isaac Sim.

## Goals

- A versioned protocol between arenad and an integration program, in any language.
- Starter kits in Rust and Python that leave an author a few functions to fill in.
- Two restore modes (names fixed by the user):
  - `exact_restore`: the part is exactly as it was. From an exact save when the part has one (fast), or by
    **replaying from the start** with the same seed and every recorded input up to the step (slower the
    later the step).
  - `approximate_restore`: from a saved state that may not capture everything (fast). Branches restored
    this way are labelled everywhere.
- Primitives: save storage, one arena clock and seed per branch, one pause point for a consistent save
  across parts, input recording for replay, tools for agents with read/write reporting, inspect and
  compare, disk accounting, a conformance check.
- Generic fallbacks so a system with no save code can still be added: a **folder** part and a **service**
  part (stop, copy, start).
- Integrations: `postgres` (generic), `folder`, `service`, `mariadb`, `erpnext`, `ridgeline-plant`
  (plant and market), `sculptured` (approvals), `isaac-sim`.
- Benchmark packages: `retail` (τ²-bench retail: its tasks, tools, data loader and gold actions, moved out
  of the core).
- The arena keeps working as today for the retail benchmark, through the new path. Old recorded runs
  still open, compare and fork.

## Non-goals

- Community hosting and publishing (design only, as already agreed). Packages install from a local folder.
- An end-to-end factory test with agents (the user: "dont worry about getting a test example").
- Running Isaac Sim here: this laptop's GPU (8 GB) is below Isaac Sim's minimum (16 GB). The `isaac-sim`
  integration is written and checked offline against a stand-in, and labelled unverified.
- Process checkpointing (CRIU): not installed, and Docker here is not an experimental build.

## Anti-goals

- Claiming a restore is exact when it is not. A part that cannot restore exactly says so, and the run says so.
- An integration that can reach into the core, or a core that imports an integration.
- An integration from a stranger that can read model keys, other parts' data, or the network at large.

## Layout

```
arena/
  core/                      arenad and the primitives (Rust); no system-specific code
  kits/rust/                 crate `arena-part`: protocol types + a server loop + helpers
  kits/python/               package `arena_part`: the same for Python
  benchmarks/
    retail/                  Rust; τ²-bench retail tasks, tools, loader, gold actions
  integrations/
    postgres/                Rust; generic
    folder/  service/        Rust; generic fallbacks
    mariadb/ erpnext/        Rust; built on the service fallback helpers in the kit
    ridgeline-plant/         Python
    sculptured/              Rust
    isaac-sim/               Python
  docs/protocol.md           the protocol, versioned
```

Rules, checked by a test in core: no crate under `integrations/` or `benchmarks/` depends on a `core/`
crate; no `core/` crate depends on anything under `integrations/` or `benchmarks/`. Core keeps Postgres
only for its own event store (the `arena` database); no world code. Rust integrations may share the
core's target directory to save disk **(my call)**; the disk is 92 % full.

## The protocol: `arena-part/1`

**Transport (my call):** JSON-RPC 2.0, one message per line, over the program's stdin and stdout. arenad
starts the program; stderr is its log. This works the same for a local program and for `docker run -i`.

**Manifest** `arena-package.toml` in each integration or benchmark folder:

```toml
kind = "integration"         # or "benchmark"
name = "postgres"            # [a-z0-9-]+, unique among installed packages
version = "0.1.0"
protocol = 1
run = { command = ["../../core/target/release/arena-postgres"] }    # or: run = { image = "…" }
needs = { gpu_vram_gb = 0, services = [] }    # e.g. services = ["host.docker.internal:5544"]
config_schema = { … }        # JSON Schema of the part's config in a world
```

`command` runs as a local program, for integrations installed from a folder you trust. `image` runs boxed
in a container: no network except the services it declares, no environment from arenad, disk only under
the save storage given to it.

**Calls from arenad to the part.** Every instance and save has an id chosen by arenad (`[a-z0-9_]+`).

| Method | Params | Result |
|---|---|---|
| `describe` | `{}` | `{name, version, protocol, restore: {exact: "save"\|"replay"\|"none", approximate: bool}, uses_clock: bool, fingerprint: {…}, tools: [ToolSchema]}` |
| `start` | `{instance, config, seed, clock_ms, from?: {save, mode: "exact"\|"approximate"}}` | `{}` |
| `call_tool` | `{instance, name, args, clock_ms}` | `{result?, error?, reads: [key], writes: [key]}` |
| `advance` | `{instance, clock_ms}` | `{changed: bool, events: [{kind, text, data}]}` |
| `save` | `{instance, save, dir}` | `{bytes, exact: bool}` |
| `inspect` | `{target: {instance}\|{save}, keys: [key]}` | `{docs: {key: doc\|null}}` |
| `diff` | `{a: target, b: target}` | `{differing: [key]}` |
| `drop_save` | `{save}` | `{}` |
| `stop` | `{instance}` | `{}` |
| `usage` | `{}` | `{bytes}` |
| `gc` (optional) | `{keep: [save]}` | `{dropped, bytes}` |
| `connect` (optional) | `{instance}` | connection info, e.g. `{url}` for `postgres` |

- `ToolSchema` is the MCP shape (`name`, `description`, `inputSchema`) plus `effect: "read"|"write"`.
  Tool names must be unique across a world; a world that has a clash is refused.
- **Keys** name a piece of a part's state, `<kind>/<id>` (for Postgres, `orders/#W1`). The part decides
  what its keys are.
- **`dir`** is a folder arenad creates for that save under `ARENA_SAVES/<part>/<save>/`. A part that keeps
  saves natively (Postgres `snap_` databases) ignores it and reports `bytes` itself.
- **`advance`** moves the part's clock to `clock_ms`. Parts with `uses_clock: false` never get it. Events
  the part reports (an alarm, a shipment arriving) are recorded as world events.
- Errors: JSON-RPC errors with a plain message. `-32001` "can't do that restore", `-32002` "unknown
  instance or save", `-32003` "fingerprint mismatch".

**Clock contract:** a part's state after `advance` to t must not depend on how the advances were split
(the part steps internally in fixed increments up to t).

**Determinism contract for `exact: "replay"`:** given the same `seed`, the same `fingerprint`, and the same
ordered inputs (`call_tool` at `clock_ms`, `advance` to `clock_ms`), the part reaches the same state and
gives the same results. The conformance check tests this.

**Benchmark calls** (a benchmark program speaks the same transport):

| Method | Params | Result |
|---|---|---|
| `describe` | `{}` | `{name, version, protocol, world: [{part, integration, config}], tasks: [{id, reason, ticket}], instructions, tools: [ToolSchema]}` |
| `seed` | `{connections: {part: info}}` | `{}`: loads the starting data into fresh instances |
| `call_tool` | `{name, args, connections}` | `{result?, error?, part, reads, writes}` |
| `gold_actions` | `{task}` | `{actions: [{name, args}]}` |

A benchmark's tools reach parts only through `connect` info; only parts with exact saves may be connected
(replay re-sends a part's own inputs, which a benchmark's direct writes are not).

## Primitives in the core

**World.** A benchmark names its world: a list of `{part, integration, config}`. Retail uses one part,
`retail` = integration `postgres`. A world is started per branch: one `start` per part, all with the
branch's seed and clock. The starting data comes from a **base save** per benchmark version, made once by
starting empty instances, calling the benchmark's `seed`, and saving. Gold saves per task are made the same
way from the base save plus the task's `gold_actions`.

**Clock and seed.** Each branch has a seed and a clock in ms. While agents work (in real time), the core
advances the clock with real time × speed (default 1) and sends `advance` to clock-using parts about every
200 ms. The clock does not count time spent saving or restoring (the world is paused; agents are not), and
each step records that pause. Each tool call records the `clock_ms` it ran at; ticks between calls are not
recorded one by one, because replay only needs "advance to t, then call" (see the clock contract).

**Pause and save.** Tool calls and clock ticks on a branch run one at a time behind the branch gate.
After each tool call the core saves, while holding the gate, so all saves for a step describe the same
moment: the part the call wrote to, and every clock part whose `advance` reported a change since its last
save (clock parts with only replay are not saved). Unchanged parts reuse their last save. The tool-call event records
`saves: {part: save}` (replacing today's single `snapshot`) plus `clock_ms`.

**Restore on a rerun.** The fork request gains `restore: "exact" | "approximate"` (default `exact`). For
each part, at the fork point:

| Part declares | `exact` | `approximate` |
|---|---|---|
| exact `save` | start from its last save | same |
| `replay` and approximate | replay from the start | start from the save with `mode: "approximate"` |
| `replay` only | replay | replay |
| approximate only | refused, naming the part | start from the save |
| none | refused | refused |

- Replay: `start` with the branch's original seed at clock 0, then for every recorded input of that part up
  to the fork point, `advance` to its `clock_ms` and repeat the call. A result that differs from the
  recorded one stops the replay and the fork fails with "replay diverged at step N" **(my call: fail, not
  continue)**.
- A replay is refused when the part's `fingerprint` differs from the one recorded at the run's start.
- The branch records `restore_mode` and `restored: [{part, how: "save"|"replay"|"approximate"}]`. The run
  page, branch list and compare show "approximate restore" on such branches, and compare explains that
  differences may come from the restore as well as the change.
- The rerun panel shows, per part, how it will be restored, and for replays an estimate from the
  original's step timings **(my call)**.

**Tools for agents.** The MCP endpoint lists the tools of every part of the branch's world (plus `handoff`)
and routes each call to its part. "Only the last agent may write" now means: tools with `effect: "write"`.

**Inspect and compare.** `GET /api/branches/{id}/rows?seq=&keys=&part=` reads keys from the save at that
step via `inspect` (no temporary database copy any more). `part` defaults to the tool's part for that step,
else the first part. Compare's "differing rows" comes from `diff` on each part's final state, keys
prefixed `part:` only when the world has more than one part. The retail check is `diff` against a gold
save, built by starting a `retail` instance and calling the task's gold actions through `call_tool`.

**Disk.** The limit counts every part's `usage` plus the size of `ARENA_SAVES`. Deleting a run calls
`drop_save` for its saves and `stop` for live instances. Startup cleanup asks each part to drop saves no
event refers to (method `gc {keep: [save]}` → `{dropped, bytes}`, optional for parts).

**Old runs.** Events with `snapshot` (no `saves`) are read as `saves: {retail: <snapshot>}`; the
`postgres` integration accepts an existing `snap_…` database name as a save id, and `branch_<id>` as an
instance id.

**Conformance check.** `arena-part-check <integration folder> [--config file]` runs the program and checks:
describe is valid and tool names well formed; start; each tool in the manifest's `[[example]]` list runs;
save → write → save → start from the first save (each supported mode) → `diff` against the first save is
empty for `exact`; `diff(a, a)` is empty; replay of the recorded inputs gives an empty `diff` against the
save (replay parts); `drop_save` lowers `usage`. It prints a report and exits non-zero on a failure. Every
integration in this repo passes it, except where it needs hardware or services not present here, which the
report says.

## The kits

Both kits implement the line protocol, method dispatch, id validation and error codes. An author writes:

- Rust: `impl Part for MyPart { fn describe; fn start; fn call_tool; fn save; fn inspect; fn diff; … }`
  and `arena_part::serve(MyPart::new())`. Async.
- Python: a class with the same methods and `arena_part.serve(MyPart())`.

Helpers in the kits, used by the fallbacks and integrations:
- `folder` saves: copy a folder into the save dir, hard-linking files unchanged since the previous save
  (rsync `--link-dest` style; ext4 has no reflinks). Saves are never modified after they are made.
- `service` helpers (Rust kit): run a Docker Compose project per instance with its own volumes; save =
  stop the project, copy its volumes with the folder helper, start it again; restore = new project from
  the copied volumes. Exact for anything whose whole state is on disk when stopped.
- Replay needs nothing from the author: the core does it.

## The integrations

| Integration | Restore | State | Tools |
|---|---|---|---|
| `postgres` | exact `save` (`CREATE DATABASE … TEMPLATE`) | one database per instance; `snap_` per save | retail pack (today's tools), `effect` per tool; keys `table/id` |
| `folder` | exact `save` | a folder per instance | `read_file`, `write_file`, `list_files`; keys `file/<path>` |
| `service` | exact `save` (stop, copy, start) | a compose project per instance | none of its own (config gives a compose file) |
| `mariadb` | exact `save` as a binlog position: restore = the instance's starting dump + the binlog applied up to the position (`mariadb-binlog`, rewritten to the new database) | one MariaDB server; one database per instance | `sql_query` (read), `sql_execute` (write); keys `table/<pk>` |
| `erpnext` | exact `save` as a binlog position (via `mariadb`) plus the site's files folder; background jobs are finished first, cache not saved | one ERPNext install; one site (and database) per instance | `get_doc`, `list_docs`, `insert_doc`, `update_doc`, `submit_doc`; keys `<Doctype>/<name>` |
| `ridgeline-plant` | exact `save` (full plant and market state plus RNG state as JSON) and replay | plant simulator in-process, on the arena clock | the `ridgeline-floor` and `ridgeline-market` tools; keys `machine/<id>`, `order/<id>`, `rfq/<id>` … |
| `sculptured` | exact `save` (SQLite online backup + data folder) | a `sculptured serve` per instance | `propose_change`, `list_changes`, `decide_change` |
| `isaac-sim` | approximate (scene `get_state`/`reset_to`) and replay | an Isaac Sim app per instance | `get_scene_state`, `set_joint_targets`, `step`; keys `prim/<path>` |

Notes:
- `ridgeline-plant` imports the simulator from `../sculptured-factory/plant` (path in config) and replaces
  its wall clock with the arena clock. The factory folder itself is not changed **(my call)**.
- `mariadb` saves are tiny and fast; restores grow with the number of changes since the instance's
  starting dump, so a restore also writes a fresh dump as the new instance's start **(my call)**. ROW
  binlog format, so applying the log never re-runs application logic.
- `isaac-sim`'s `fingerprint` is GPU name, driver version and Isaac Sim version. Its replay is exact only on
  the same fingerprint, per NVIDIA's determinism notes, and only for rigid bodies and articulations.

## Arena UI and Sculptured Tests UI

- The rerun panel gets a restore choice (`Exact` default, `Approximate`) with one line per part saying how
  it will be restored and, for replays, about how long.
- Branches restored approximately carry an "Approximate restore" mark in the branch list, run header and
  compare.
- The step inspector's before/after works for any part's keys (it already reads by key).
- Both UIs: `arena/ui` and Sculptured's `web/src/tests/` (branch `tests-arena`).

## Left to the builder

Exact JSON-RPC framing details beyond one message per line; how the kits structure async; the Compose file
of the `erpnext` integration (start from `../sculptured-factory/erp/compose.yml`); port allocation for
per-instance services; how the plant's state is serialised.
