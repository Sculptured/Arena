# Integrations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Any part of a system (a database, ERPNext, a simulator, a Sculptured server, Isaac Sim) can be part of an arena world, saved at every step and restored on a rerun with `exact_restore` or `approximate_restore`. Each part is an isolated integration program built on primitives the arena supplies. Benchmarks are separate packages.

**Architecture:**
- arenad starts integration and benchmark programs from `arena-package.toml` manifests and talks to them over JSON-RPC lines on stdio (`arena-part/1`).
- The core supplies the hub, the clock, saves, replay, restore modes, recording and the conformance check.
- Kits (Rust crate `arena-part`, Python package `arena_part`) implement the protocol and the helpers: folder copies, Postgres templates, MariaDB binlog positions, Compose services.
- The retail world becomes `integrations/postgres` (generic) plus `benchmarks/retail`.

**Tech Stack:**
- Rust nightly 1.99, tokio, serde_json, axum 0.8, tokio-postgres, async-trait, toml.
- Python 3.12 with uv; stdlib only for the kit.
- Docker images already present here: `postgres:17-alpine`, `mariadb:11.8`, `frappe/erpnext:v16.53.0`, `redis:6.2-alpine`.

**Spec:** `arena/docs/specs/2026-10-31-integrations-design.md`. Read it before any task: protocol tables, restore table and integration table are binding.

## Global Constraints

- **Protocol** `arena-part/1`: JSON-RPC 2.0, one JSON object per line on stdin/stdout, stderr is log. Method names and params exactly as the spec's tables. Error codes: `-32001` can't do that restore, `-32002` unknown instance or save, `-32003` fingerprint mismatch, `-32601` method not found, `-32602` invalid params.
- **Ids:** instance and save ids match `^[a-z0-9_]{1,63}$`; package names match `^[a-z0-9-]{1,40}$`; keys are `<kind>/<id>` strings.
- **Isolation:**
  - Nothing under `integrations/`, `benchmarks/` or `kits/` depends on a `core/` crate.
  - No `core/` crate depends on anything under `integrations/` or `benchmarks/`.
  - `core/crates/parts` may depend on `kits/rust` (protocol types only).
- **Cargo:**
  - A second workspace at `arena/Cargo.toml` with `exclude = ["core"]` holds `kits/rust`, `benchmarks/*` and the Rust integrations.
  - `arena/.cargo/config.toml` sets `target-dir = "core/target"`, so both workspaces share one target directory. The disk is 92 % full: never create another target dir, never `cargo clean` the shared target without asking.
- **Binaries in manifests:** `run.command` entries may contain `${ARENA_BIN_DIR}`. arenad substitutes it from env `ARENA_BIN_DIR`, default `<arena>/core/target/release`. Tests set it to `core/target/debug`.
- **Package roots:** env `ARENA_PACKAGES`, a `:`-separated list of folders whose subfolders hold `arena-package.toml`. Default `<arena>/integrations:<arena>/benchmarks`.
- **Saves storage:** env `ARENA_SAVES`, default `<arena>/core/saves`, ignored by git. Per save: `ARENA_SAVES/<package>/<save>/`.
- **Postgres** for the core's own store and the `postgres` integration: `ARENA_PG_URL`, default `postgres://arena:arena@127.0.0.1:5544/postgres` (running now).
- **Don't touch:**
  - `../sculptured-factory` (read only);
  - other Docker containers and volumes than the ones a test creates and removes;
  - the user's running Sculptured (:4410) and runners;
  - the arenad on :4510 (don't restart it while tests run).
- **Tests never call a real model.** Tests that need Docker services or hardware not always present are `#[ignore]`d with the prefix `live_` (Rust) or marked `@pytest.mark.live` (Python). The task says which ones to run once for real.
- **Copy rules:**
  - UI text in plain words, matching the existing arena UI style.
  - Old recorded runs must still open, compare and fork.
- Commit after each task with a message ending in `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **An integration program crashes or exits mid-run.** The branch ends `infra_error` with "the <part> integration stopped: <stderr tail>". The next use respawns it. Pinned by `hub_respawns_after_crash` (Task 3) and `part_crash_ends_branch_infra_error` (Task 7).
2. **Two branches use one integration program at once.** Their instances and saves never mix. Pinned by `two_branches_one_program` (Task 7).
3. **`exact` is requested for a world with a part that can't restore exactly.** 409, with a message naming the part and suggesting approximate. Pinned by `exact_refused_names_part` (Task 9).
4. **Runs recorded before this change** (events with `snapshot`, no `saves`) open, show rows, compare and fork. Pinned by `old_run_still_forks` (Task 8).
5. **Deleting a run** drops every part's saves and live instances, and the disk usage figure falls. Pinned by `delete_run_drops_all_part_saves` (Task 8).

---

### Task 1: Rust kit core, protocol doc, packages workspace

**Files:**
- Create: `arena/Cargo.toml` (workspace, `exclude = ["core"]`, members `kits/rust`), `arena/.cargo/config.toml`, `arena/.gitignore` additions (`core/saves/`)
- Create: `kits/rust/Cargo.toml` (crate `arena-part`, lib), `kits/rust/src/{lib.rs,proto.rs,serve.rs,error.rs}`
- Create: `kits/rust/examples/counter.rs`, `kits/rust/examples/counter/arena-package.toml`
- Create: `docs/protocol.md`
- Test: `kits/rust/tests/serve.rs`

**Interfaces:**
- Produces, in `arena_part::proto`:
  - Serde types matching the spec tables: `Describe`, `Restore {exact: ExactKind, approximate: bool}`, `ExactKind {Save, Replay, None}` (serialized lowercase), `ToolSchema {name, description, #[serde(rename="inputSchema")] input_schema, effect: Effect}`, `Effect {Read, Write}`.
  - `StartParams {instance, config: Value, seed: u64, clock_ms: i64, from: Option<FromSave>}`, `FromSave {save, mode: Mode}`, `Mode {Exact, Approximate}`.
  - `CallToolParams {instance, name, args: Value, clock_ms: i64}`, `ToolOutput {result: Option<Value>, error: Option<String>, reads: Vec<String>, writes: Vec<String>}`.
  - `AdvanceParams {instance, clock_ms}`, `Advanced {changed: bool, events: Vec<WorldEvent>}`, `WorldEvent {kind, text, data: Value}`.
  - `SaveParams {instance, save, dir: String}`, `Saved {bytes: u64, exact: bool}`.
  - `Target` (untagged: `Instance {instance}` | `Save {save}`), `InspectParams {target, keys}`, `Docs {docs: Map<String, Value>}`, `DiffParams {a, b}`, `Differing {differing: Vec<String>}`.
  - `IdParams {instance}` / `SaveId {save}`, `Usage {bytes: u64}`, `GcParams {keep: Vec<String>}`, `GcResult {dropped: u64, bytes: u64}`.
  - Benchmark side: `BenchDescribe {name, version, protocol, world: Vec<WorldPart {part, integration, config}>, tasks: Vec<TaskInfo {id, reason, ticket}>, instructions: String, tools: Vec<ToolSchema>}`, `SeedParams {connections: Map<String, Value>}`, `BenchCallTool {name, args, connections}`, `BenchToolOutput {result, error, part: String, reads, writes}`, `GoldParams {task}`, `GoldActions {actions: Vec<Action {name, args}>}`.
  - `PROTOCOL: u32 = 1`, `fn valid_id(&str) -> bool`, `fn valid_package_name(&str) -> bool`.
- Produces, in `arena_part`:
  - `PartError {code: i64, message: String}` with constructors `cant_restore`, `unknown`, `fingerprint`, `invalid`, `internal`.
  - `#[async_trait] trait Part: Send + Sync + 'static` with: `describe`, `start`, `call_tool`, `advance` (default: `{changed:false, events:[]}`), `save`, `inspect`, `diff`, `drop_save`, `stop`, `usage`, `gc` (default `{0,0}`), `connect` (default: error `-32601`). Each takes its params struct and returns `Result<_, PartError>`.
  - `#[async_trait] trait Benchmark`: `describe`, `seed`, `call_tool`, `gold_actions`.
  - `async fn serve<P: Part>(p: P) -> anyhow::Result<()>` and `async fn serve_benchmark<B: Benchmark>(b: B)`. Both read stdin lines, run each request on its own task (concurrent), and write responses whole-line under a mutex. They validate ids in params, answer -32601/-32602, and exit when stdin closes.
  - `fn serve_on<P, R: AsyncBufRead, W: AsyncWrite>(p, r, w)` for tests.
- `counter` example: one integer per instance; tool `add {n}` (write, key `counter/value`), tool `get` (read); `uses_clock: true`, and `advance` adds 1 per whole second of clock. Exact save to `dir/state.json`, plus replay. It is what core tests spawn.

- [ ] **Step 1: Write failing tests** in `kits/rust/tests/serve.rs`, driving `serve_on` with in-memory pipes and the counter part:
  - `describe_reports_protocol_1`: `result.protocol == 1`, and `tools[0].inputSchema` exists.
  - `unknown_method_is_32601`, `bad_instance_id_is_32602` (instance `"Bad Id"`).
  - `save_then_start_from_save_restores`: add 5, save `s1`, add 2, start `i2` from `s1` exact, get gives 5.
  - `requests_answered_concurrently`: two ids in flight both get answered, ids matched.
  - `advance_is_split_independent`: advancing to 2500 in one call or five calls gives the same `get`.
- [ ] **Step 2:** `cd arena && cargo test -p arena-part`. Expect a compile failure.
- [ ] **Step 3:** Implement. `docs/protocol.md` holds the spec's protocol section (transport, manifest, both method tables, error codes, clock and determinism contracts) plus one worked example exchange.
- [ ] **Step 4:** `cargo test -p arena-part` passes, and `cargo build --example counter` works.
- [ ] **Step 5:** Commit "kits: Rust arena-part kit and protocol".

### Task 2: Python kit

**Files:**
- Create: `kits/python/pyproject.toml` (package `arena_part`, no dependencies), `kits/python/arena_part/{__init__.py,serve.py,errors.py,folder.py,clock.py}`
- Create: `kits/python/examples/counter.py`, `kits/python/examples/arena-package.toml`
- Test: `kits/python/tests/test_serve.py`, `tests/test_folder.py`

**Interfaces:**
- `class Part`: methods `describe()`, `start(instance, config, seed, clock_ms, from_)` (`from_` is `None` or `{"save", "mode"}`), `call_tool(instance, name, args, clock_ms)`, `advance(instance, clock_ms)`, `save(instance, save, dir)`, `inspect(target, keys)`, `diff(a, b)`, `drop_save(save)`, `stop(instance)`, `usage()`, `gc(keep)`, `connect(instance)`. Defaults as in the Rust kit.
- `serve(part, stdin=sys.stdin, stdout=sys.stdout)`: serial, one request at a time.
- `PartError(code, message)` plus helpers `cant_restore`, `unknown`, `fingerprint`.
- `folder.copy_save(src: Path, dst: Path, prev: Path | None) -> int`: copies `src` into `dst`, hard-linking files whose size and mtime match `prev`'s copy. Returns bytes newly written.
- `folder.restore(save_dir: Path, dst: Path)`: copies, never links, so the live folder can't modify a save.
- `clock.FixedStep(step_ms=100)`: `advance_to(clock_ms, fn(step_ms))` calls `fn` once per whole step and keeps the remainder, which gives the clock contract.

- [ ] **Step 1:** Write the same five tests as Task 1 (pytest, driving `serve` with `io.StringIO`), plus:
  - `test_copy_save_links_unchanged`: the second save of an unchanged file shares its inode with the first.
  - `test_restore_never_links`.
- [ ] **Step 2:** `cd kits/python && uv run --with pytest pytest -q` fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** The tests pass.
- [ ] **Step 5:** Commit "kits: Python arena_part kit".

### Task 3: Core `parts` crate: manifests, client, hub, isolation check

**Files:**
- Create: `core/crates/parts/{Cargo.toml,src/lib.rs,src/manifest.rs,src/client.rs,src/hub.rs}`; add to `core/Cargo.toml` members
- Test: `core/crates/parts/tests/{hub.rs,isolation.rs}`

**Interfaces:**
- Consumes: `arena_part::proto::*`, plus the `counter` examples from Tasks 1 and 2.
- Produces:
  - `Manifest {kind: Kind (Integration|Benchmark), name, version, protocol: u32, run: Run (Command(Vec<String>) | Image(String)), needs: Needs {gpu_vram_gb: u32, services: Vec<String>, network: Option<String>}, dir: PathBuf, examples: Vec<Value>}`.
  - `Manifest::load(dir) -> Result<Manifest>`: refuses a bad name or a protocol other than 1.
  - `Registry::scan(roots: &[PathBuf]) -> Result<Registry>`: duplicate names are an error. Also `get(&str) -> Option<&Manifest>`, `benchmarks()`, `integrations()`.
  - `Client::spawn(m: &Manifest, bin_dir: &Path) -> Result<Client>`.
    - Command mode: resolve `${ARENA_BIN_DIR}` and paths relative to `m.dir`, clear the environment except `PATH`, `HOME`, `ARENA_PG_URL`, `ARENA_SAVES`.
    - Image mode: `docker run -i --rm --network none` (or `--network host` when `needs.network == "host"`, logged), `-v <ARENA_SAVES>/<name>:/saves`, no env.
  - `Client::call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T, CallError>`.
    - `CallError::{Part {code, message}, Stopped (String, with the last 20 stderr lines), Timeout}`; the default timeout is 600 s.
    - Responses are matched by id; concurrent calls are allowed.
  - `Hub::new(registry: Registry, bin_dir: PathBuf, saves_root: PathBuf)`.
    - `async fn client(&self, name) -> Result<Arc<Client>>`: lazy, and respawns if stopped.
    - `async fn describe(&self, name) -> Result<Describe>` and `async fn bench_describe(&self, name) -> Result<BenchDescribe>`, cached per spawn.
    - `fn save_dir(&self, package, save) -> PathBuf`: creates it.
    - `fn saves_bytes(&self) -> u64`.

- [ ] **Step 1: Write failing tests:**
  - `manifest_rejects_bad_name`, `registry_rejects_duplicates`.
  - `hub_spawns_rust_counter_and_saves`: build the example via `cargo build --example counter -p arena-part` in a OnceLock helper, point a temp root at a manifest using `${ARENA_BIN_DIR}/examples/counter`, then start, add, save.
  - `hub_spawns_python_counter`: command `["uv","run","--project","<kits/python>","python","<examples/counter.py>"]`.
  - `hub_respawns_after_crash`: kill the child's pid; the next `client()` works.
  - `stopped_error_carries_stderr`.
  - `isolation.rs`: runs `cargo metadata --format-version 1` for both workspaces and asserts both isolation rules from Global Constraints.
- [ ] **Step 2:** `cd core && cargo test -p parts` fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** The tests pass.
- [ ] **Step 5:** Commit "core: parts hub (manifests, client, isolation check)".

### Task 4: Conformance check `arena-part-check`

**Files:**
- Create: `core/crates/parts/src/bin/arena-part-check.rs`, `core/crates/parts/src/check.rs`
- Test: `core/crates/parts/tests/check.rs`

**Interfaces:**
- `async fn check(hub: &Hub, name: &str, config: Value) -> Report`.
  - `Report {package, passed: bool, steps: Vec<Step {name, ok, detail}>, skipped: Vec<String>}`, serialized as JSON.
- Steps, in order (spec "Conformance check"):
  1. Describe is valid, tool names are unique and well formed, and `protocol == 1`.
  2. Start `chk_a`.
  3. Run each manifest `[[example]]` (`{tool, args}`).
  4. Save `chk_s1`.
  5. Run a write example.
  6. Save `chk_s2`.
  7. For each supported mode, start `chk_b` from `chk_s1` and expect `diff(chk_b, chk_s1)` to be empty (exact only; approximate is reported, not failed).
  8. `diff(chk_s1, chk_s1)` is empty.
  9. Replay parts only: start `chk_r` with the same seed, re-send the recorded inputs, and expect `diff(chk_r, chk_s2)` to be empty.
  10. `drop_save` both saves and expect `usage` not to grow.
  11. Stop all instances.
- CLI: `arena-part-check <package dir> [--config file.json]` prints the report and exits 1 on failure.

- [ ] **Step 1:** Write failing tests:
  - `counter_passes`, for both the Rust and the Python counter.
  - `broken_restore_fails`: a test-only example `counter --broken-restore` that restores 0. The report fails at step 7.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** They pass.
- [ ] **Step 5:** Commit "core: arena-part-check".

### Task 5: Postgres helper in the kit, and the generic `postgres` integration

**Files:**
- Create: `kits/rust/src/helpers/{mod.rs,pg.rs}` behind feature `postgres` (tokio-postgres)
- Create: `integrations/postgres/{Cargo.toml,arena-package.toml,src/main.rs}` (binary `arena-postgres`)
- Test: `integrations/postgres/tests/part.rs`

**Interfaces:**
- `arena_part::helpers::pg::Templates::new(admin_url)`:
  - `create_from(new, template)`, `drop(name)`, `size(name)`, `list(prefix)`;
  - a per-database lock, the same rule as today's `world::snapshot::Worlds` (no connection open while copying).
- `postgres` integration:
  - Instance → database `pi_<instance>`; save → `ps_<save>`.
  - Start, no `from`: empty database, then `config.init_sql` if present. Start with `from`: template copy.
  - Restore: exact save; approximate false.
  - `connect` → `{url}`.
  - `inspect` keys `<table>/<pk>` → `row_to_json`, for tables with a single-column primary key.
  - `diff` compares every table's rows by primary key.
  - `usage` sums `pi_`/`ps_` sizes. `gc {keep}` drops `ps_` saves not kept and `pi_` instances not live.
  - No tools by default; with `config.sql_tools: true`, `sql_query` (read) and `sql_execute` (write; reads and writes reported as `t/*`).
  - **Old-run compatibility:** a save id or instance id that names an existing database with prefix `snap_`, `branch_`, `world_base` or `gold_` is used as is.

- [ ] **Step 1:** Write failing tests:
  - `save_restore_roundtrip`.
  - `diff_reports_changed_row` (key `t/1`).
  - `legacy_snap_name_accepted`: create `snap_test_x`, inspect it as a save.
  - `gc_keeps_listed`.
  - The conformance check passes, using config `{init_sql: "CREATE TABLE t(id int primary key, v text)", sql_tools: true}`. `sql_tools: true` turns on two tools, `sql_query` (read) and `sql_execute` (write), which are off by default because benchmarks own the tools. The manifest `[[example]]` entries use them.
- [ ] **Step 2:** Run `cd arena && cargo test -p arena-postgres`; it fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** It passes. Every test drops its databases, as the leakcheck rule in the README requires.
- [ ] **Step 5:** Commit "integrations: generic postgres".

### Task 6: `benchmarks/retail`

**Files:**
- Create: `benchmarks/retail/{Cargo.toml,arena-package.toml,src/main.rs,src/tools.rs,src/load.rs,src/tasks.rs}` (binary `arena-retail`)
- Move: `core/crates/world/src/tools.rs` → `benchmarks/retail/src/tools.rs`, unchanged in behaviour. Loading moves from `schema.rs`, tasks from `arenad/src/tasks.rs`, and the gold-action list from `gold.rs`.
- Move: `data/retail/*` stays where it is. The benchmark reads it from `config.data_dir`, which defaults to `<package dir>/../../data/retail`.
- Test: port `core/crates/world/tests/{tools.rs,load.rs}` to `benchmarks/retail/tests/`

**Interfaces:**
- `describe`:
  - `world = [{part: "retail", integration: "postgres", config: {}}]`;
  - `tasks` are the 10 demo tasks, with `reason` and `ticket` (today's `Tasks::ticket` text);
  - `instructions` is `policy.md`;
  - `tools` are today's schemas plus `effect` (the write tools are the current `WRITE_TOOLS`).
- `seed {connections: {retail: {url}}}`: creates the three tables and loads `db.json`, as `world::schema::load_base` does today.
- `call_tool {name, args, connections}` runs today's `tools::call` against `connections.retail.url` and returns `part: "retail"` with reads and writes.
- `gold_actions {task}` returns `evaluation_criteria.actions`.

- [ ] **Step 1:** Port the tests so they go through the benchmark's functions against a database created by the test (template from a seeded one). Add `gold_actions_for_27_nonempty`.
- [ ] **Step 2:** `cargo test -p arena-retail` fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** It passes.
- [ ] **Step 5:** Commit "benchmarks: retail as its own package".

### Task 7: arenad runs worlds through the hub

**Files:**
- Modify: `core/crates/arenad/src/{app.rs,main.rs,runner.rs,exec.rs,mcp.rs,model.rs,store.rs,api.rs (tasks/health only),agents/*.rs (tool list source only)}`
- Delete: `core/crates/world` (code moved in Tasks 5 and 6), `arenad/src/tasks.rs`. `store.rs` gets its own small Postgres admin helper in place of `world::pg::Admin`.
- Test: adapt `core/crates/arenad/tests/{runner.rs,mcp.rs,api.rs,cli.rs,open.rs,store.rs}`; a new `tests/support/packages.rs` builds `arena-postgres` and `arena-retail` (debug) once and sets `ARENA_BIN_DIR` and `ARENA_PACKAGES`.

**Interfaces:**
- Consumes: `parts::Hub`, the benchmark `retail`, the integration `postgres`.
- `AppState` holds `hub: Arc<Hub>` and `benchmark: String` (env `ARENA_BENCHMARK`, default `retail`).
- `app::World` (new, `arenad/src/world.rs`):
  - `World::start(hub, bench: &BenchDescribe, branch_id, seed, from: WorldFrom) -> Result<World>`.
  - `WorldFrom::{Base, Saves(BTreeMap<part, (save, Mode)>), Replay…}`; Replay is added in Task 9.
  - `World::call_tool(name, args, clock_ms) -> (ToolOutput with part)`: routes to the benchmark's tool, or to the part that lists it.
  - `World::save_changed(written_part: Option<&str>) -> BTreeMap<String, String>`.
  - `World::stop()`.
  - Instance ids are `b_<branch>_<part>`; save ids are `s_<uuid simple>`.
- **Base save:** `ensure_base(hub, bench) -> BTreeMap<part, save>`, named `base_<bench>_<version with . → _>_<part>`, created once by starting empty instances, calling `seed`, saving and stopping.
- **Gold save:** `ensure_gold(task)`, named `gold_<task>_<part>`, built from base plus `gold_actions` through `call_tool`. The check is `diff(instance, gold save)` per part; `CheckPayload.differing_rows` is unchanged in shape.
- **Payloads:**
  - `RunStart` payload: `{task_id, team, benchmark, world, seed, fingerprints: {part: Value}, saves: {part: save}}`.
  - `ToolCallPayload` gains `part: Option<String>`, `saves: Option<BTreeMap<String, String>>` and `clock_ms: Option<i64>`. `snapshot` is still written, as `saves["retail"]` when that part exists, so the old UIs keep working.
- **Tools:** the MCP `tools/list` is the world's tools plus `handoff`. A write-tool refusal for non-last agents uses `effect == Write`. A tool-name clash between the benchmark and the parts refuses the run start with "two tools are called <name>".
- `/api/tasks` and `/api/health` read from the benchmark describe.

- [ ] **Step 1:** Write the new failing tests:
  - `run_records_saves_per_part`: after a write, the tool_call payload has `saves.retail` and `part == "retail"`.
  - `two_branches_one_program`: two runs concurrently; each branch's final dump is its own.
  - `part_crash_ends_branch_infra_error`: a hub with a test integration that exits on its second call.
  - `tool_name_clash_refused`.
  - Then adapt the existing tests to the new support module.
- [ ] **Step 2:** `cd core && cargo test -p arenad`. The new tests fail.
- [ ] **Step 3:** Implement. Keep `exec.rs` the single place where calls are executed, recorded and saved.
- [ ] **Step 4:** `cargo test -p arenad` all passes, and `cargo clippy -p arenad -p parts` is clean.
- [ ] **Step 5:** Commit "arenad: run worlds through integrations".

### Task 7a: Code-quality pass on Tasks 1–6

The user asked (2026-10-31) that the code stay modular, DRY and in short functions. `arena/AGENTS.md` and `arena/clippy.toml` (threshold 30) now say so.

**Files:** non-test code in `kits/rust`, `kits/python`, `core/crates/parts`, `core/crates/arenad` (pre-existing long functions such as `fork::plan_fork`, `main`, `mcp::handle`, `open::drive`, `parts::client::spawn_with_env`), `integrations/postgres` and `benchmarks/retail` (including the copied `tools.rs`), plus `[lints]` wiring.

- [ ] **Step 1:** Add `[workspace.lints.clippy] too_many_lines = "warn"` to `arena/Cargo.toml` and `[lints] workspace = true` to every package crate. Task 7 does the same for `core`.
- [ ] **Step 2:** List the functions over 30 lines with `cargo clippy --all-targets -- -W clippy::too_many_lines` in both workspaces. Refactor each into helpers around real responsibilities. Remove duplicated logic. Behaviour stays identical: the existing tests are the proof (`benchmarks/retail`'s 68-task gold sweep covers `tools.rs`).
- [ ] **Step 3:** Run every test suite of the touched crates, plus the Python kit's tests. Clippy shows no `too_many_lines` warnings in non-test code, except ones listed in the report with a reason.
- [ ] **Step 4:** Commit "quality: short functions and shared helpers in kits, parts, postgres, retail".

### Task 8: Rows, compare, delete, disk, cleanup and old runs through parts

**Files:**
- Modify: `core/crates/arenad/src/{api.rs,compare.rs,gc.rs,store.rs,fork.rs (restore lookup only)}`
- Test: `core/crates/arenad/tests/{api.rs,disk.rs,fork.rs}`

**Interfaces:**
- `fn saves_at(events: &[Event], seq: i64, base: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, NoSave {part, seq}>`: per part, the last `saves[part]` (or legacy `snapshot` → `retail`) at or before `seq`. Base comes from `RunStart.saves` (legacy run: `{"retail": "world_base"}`). A committed write to a part without a save makes that part unrestorable until its next save, as today.
- `GET /api/branches/{id}/rows?seq=&keys=&part=`: `inspect {save}` on that part. `part` defaults to the step's `part`, else the world's first part. The 50-key limit stays, and key validation is the part's (`-32602` → 400).
- `GET /api/compare` takes differing rows from `diff` of both branches' final saves per part. Keys are prefixed `part:` only when the world has more than one part.
- **Delete run:** `drop_save` for every save its events and RunStart name (except base and gold), then `stop` live instances.
- **Disk:** the limit compares `sum(usage of each integration in use) + hub.saves_bytes()`.
- **Startup cleanup:** for each integration, `gc {keep: every save named by any event or RunStart, plus base_* and gold_*}`. Integrations without `gc` are skipped and logged.

- [ ] **Step 1:** Write failing tests:
  - `rows_via_inspect_before_after`.
  - `compare_differing_rows_unchanged_shape`.
  - `delete_run_drops_all_part_saves` (usage after < before).
  - `old_run_still_forks`: insert a legacy run by hand (events with `snapshot: "snap_…"`, a real snap database made from base), then rows, compare and fork all succeed.
  - `startup_gc_keeps_named_saves`.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** All arenad tests pass.
- [ ] **Step 5:** Commit "arenad: rows, compare, delete and disk through parts".

### Task 9: Clock, restore modes, replay, fingerprints

**Files:**
- Modify: `core/crates/arenad/src/{world.rs,exec.rs,runner.rs,fork.rs,api.rs,model.rs,store.rs}`
- Test: `core/crates/arenad/tests/{restore.rs (new),fork.rs}`; a test integration `core/crates/arenad/tests/support/parts/` with manifests for the Rust `counter` example in three flavours: `exact` (save), `replay_only`, `approx_and_replay`. The counter example takes `--flavour <name>` (Task 1) to change what `describe` reports; test manifests pick it in `run.command`.

**Interfaces:**
- `BranchClock {base_ms: i64, speed: f64}`:
  - `now_ms()` counts real time × speed only while running;
  - `pause()` / `resume()` wrap saves and restores; paused time is recorded on the step as `paused_ms`.
  - A ticker task per live branch sends `advance` to `uses_clock` parts every 200 ms behind the branch gate. The `changed` flags are kept for `save_changed`. `WorldEvent`s are appended as a new `EventKind::WorldEvent` (`"world_event"`, payload `{part, kind, text, data, clock_ms}`).
- The fork request gains `restore: "exact" | "approximate"` (default exact). The plan, per the spec's restore table, is `fn restore_plan(describes, mode) -> Result<BTreeMap<part, How>, Refused {part, reason}>`, with `How::{Save(Mode), Replay}`.
- **Replay:** start with the run's seed at clock 0. For each recorded `tool_call` of that part (`part` field) before the fork point, `advance` to its `clock_ms` (clock parts), then `call_tool` and compare the result and error with the recorded ones. Finally advance to the fork point's clock. A mismatch → `ForkError::ReplayDiverged {part, seq}` → 409 "replay of <part> diverged at step N, so it can't be restored exactly".
- **Fingerprints:** recorded in RunStart. Replay refuses on a mismatch with `-32003` → 409 "the <part> integration is running on a different machine or version than the original".
- **Branch columns:** `restore_mode text`, `restored jsonb` (`[{part, how: "save"|"replay"|"approximate"}]`), added by `ALTER TABLE … ADD COLUMN IF NOT EXISTS`. They are exposed on `Branch` and in `/api/runs`, `/api/branches/{id}` and compare (`a.restore_mode`, `b.restore_mode`).
- `GET /api/branches/{id}/restore-plan?at_seq=&restore=` → `{parts: [{part, how, estimate_seconds?}]}`. The replay estimate is the sum of the original's tool-call durations up to the fork point.

- [ ] **Step 1:** Write failing tests in `restore.rs`:
  - `exact_refused_names_part` (an approximate-only part).
  - `approximate_marks_branch`.
  - `replay_only_restores_exactly`.
  - `replay_divergence_fails`: the counter flavour `nondeterministic` adds a random number.
  - `fingerprint_mismatch_refused`.
  - `clock_advances_and_pauses_during_save`.
  - `restore_plan_endpoint`.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** All arenad tests pass.
- [ ] **Step 5:** Commit "arenad: arena clock, exact and approximate restore, replay".

### Task 10: Folder and Compose helpers; `folder` and `service` integrations

**Files:**
- Create: `kits/rust/src/helpers/{folder.rs,compose.rs}` (features `folder` (default), `compose`)
- Create: `integrations/folder/`, `integrations/service/` (binaries `arena-folder`, `arena-service`)
- Test: `kits/rust/tests/folder.rs`; `integrations/folder/tests/part.rs`; `integrations/service/tests/live_service.rs` (ignored)

**Interfaces:**
- `helpers::folder`:
  - `copy_save(src, dst, prev: Option<&Path>) -> io::Result<u64>`: hard-links unchanged files (same size and mtime) from `prev`, and returns new bytes;
  - `restore(save, dst)`, which copies;
  - `size(path)`.
- `helpers::compose::Project::new(name, compose_file, env)`: `up()`, `stop()`, `start()`, `down_with_volumes()`, `volume_paths() -> Vec<(String, PathBuf)>` (read via `docker volume inspect`; data copied through a throwaway `alpine` container if not readable).
- `folder` integration:
  - config `{root?}`; instance folder `<saves root>/folder/live/<instance>`;
  - tools `list_files` (read), `read_file` (read, key `file/<path>`), `write_file` (write);
  - paths may not leave the instance folder;
  - exact save.
- `service` integration:
  - config `{compose_file, volumes: [name]}`;
  - instance = project `arena_<instance>`;
  - save = stop, `copy_save` each volume, start;
  - start from save = new project whose volumes are filled from the save;
  - no tools; exact save.

- [ ] **Step 1:** Write failing tests:
  - `copy_save_links_unchanged`, `restore_is_a_copy`.
  - The folder integration passes `arena-part-check`.
  - `write_outside_root_refused` (`../x`).
  - `live_service_roundtrip`, using a compose file with one `redis:6.2-alpine` service and a volume: set a key, save, change it, restore, read the key back. Run it once for real.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** They pass. `cargo test -p arena-service -- --ignored live_` passes once. Remove the test's containers and volumes.
- [ ] **Step 5:** Commit "integrations: folder and service fallbacks".

### Task 11: MariaDB binlog helper and `mariadb` integration

**Files:**
- Create: `kits/rust/src/helpers/mariadb.rs` (feature `mariadb`, mysql_async or the `mariadb` CLI via `docker exec`; my call: CLI via `docker exec` so no new driver crate)
- Create: `integrations/mariadb/` (binary `arena-mariadb`)
- Test: `integrations/mariadb/tests/live_mariadb.rs` (ignored; uses `mariadb:11.8`)

**Interfaces:**
- `helpers::mariadb::Server::ensure(container, port, root_password)`: starts `mariadb:11.8` with `--log-bin=binlog --binlog-format=ROW --server-id=1 --binlog-row-image=FULL`, on `127.0.0.1:<port>` (default 3307), if not running.
- `Position {file: String, pos: u64}` and `server.position() -> Position` (`SHOW MASTER STATUS` / `SHOW BINARY LOG STATUS`).
- `server.dump(db, path)` and `server.load(path, db)`.
- `server.apply_log(from_db, from: Position, to: Position, into_db)`: `mariadb-binlog --database=<from_db> --start-position… --stop-position…` across the files in between, with `--rewrite-db='<from_db>-><into_db>'`, piped into `mariadb`.
- `mariadb` integration:
  - instance = database `mi_<instance>`;
  - at start, record `start_dump` (a file under the instance's save storage) and `start_pos`;
  - save = `{db, start_dump, start_pos, to_pos}` as JSON in the save dir (bytes = JSON size, since the dump is shared and counted once by usage);
  - start from save = load `start_dump` into the new db, apply the log `start_pos..to_pos`, then write a new start dump and position for the new instance;
  - tools `sql_query` (read), `sql_execute` (write; reads and writes are reported as `table/<pk>` when the statement is a single-table insert, update or delete with a primary key, else `table/*`);
  - `inspect` `table/<pk>`; `diff` per table by primary key.

- [ ] **Step 1:** Write `live_mariadb_roundtrip` (insert, save s1, update, save s2, restore s1 into a new instance, the row equals the old value; restore s2 equals the new value) and `live_mariadb_restore_of_restore` (fork of a restored instance). The conformance check passes with the `live_` flag.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** `cargo test -p arena-mariadb -- --ignored live_` passes. Run it once for real, then remove the test container.
- [ ] **Step 5:** Commit "integrations: mariadb with binlog saves".

### Task 12: `erpnext` integration

**Files:**
- Create: `integrations/erpnext/` (binary `arena-erpnext`, `compose.yml` adapted from `../sculptured-factory/erp/compose.yml` with MariaDB binlog flags and loopback ports 8180/3308, project name `arena_erpnext`)
- Test: `integrations/erpnext/tests/live_erpnext.rs` (ignored)

**Interfaces:**
- One ERPNext install, brought up on first `start`.
  - The base site `base.localhost` is created once with `bench new-site` and `install-app erpnext`.
  - Its database start dump and position are recorded through the mariadb helper.
- **Instance:**
  - site `<instance>.localhost`: a new database, filled by load plus apply_log;
  - its `site_config.json` is written with its own db name and password, and a MariaDB user is created with grants;
  - the site files folder is restored with the folder helper.
- **Save:**
  - wait until the site's RQ queues are empty (`bench --site <s> show-pending-jobs` or Redis `LLEN`, timeout 60 s, else an error);
  - then record the binlog position plus `copy_save` of `sites/<site>/{public,private}/files`.
- **Tools** (REST with an Administrator API key stored in the base site; every request sends a `Host` header naming the instance's site):
  - `get_doc {doctype, name}` (read),
  - `list_docs {doctype, filters?, fields?, limit?}` (read),
  - `insert_doc {doc}` (write),
  - `update_doc {doctype, name, fields}` (write),
  - `submit_doc {doctype, name}` (write).
  - Keys are `<Doctype>/<name>`.
- `inspect` uses REST; `diff` dumps the doctypes listed in `config.diff_doctypes` (default `["Item","Supplier","Customer","Purchase Order","Sales Order","Work Order","Journal Entry"]`).

- [ ] **Step 1:** Write `live_erpnext_roundtrip`: insert an Item, save, update it, restore into a new instance, and `get_doc` shows the original. Then run the conformance check.
- [ ] **Step 2:** Run it; it fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the live test once for real. Note the first-time setup time and the save and restore times in the report. Afterwards `docker compose -p arena_erpnext down -v`, and check the disk before and after.
- [ ] **Step 5:** Commit "integrations: erpnext (site per instance, binlog saves)".

### Task 13: `ridgeline-plant` integration (Python)

**Files:**
- Create: `integrations/ridgeline-plant/{arena-package.toml,pyproject.toml,ridgeline_part/__init__.py,ridgeline_part/part.py,ridgeline_part/clocked.py}`
- Test: `integrations/ridgeline-plant/tests/test_part.py`

**Interfaces:**
- Imports `ridgeline_plant` from `config.plant_path` (default `../../../sculptured-factory/plant`), read only.
- `clocked.ClockedPlant` wraps `Plant`. It replaces the module's `time.monotonic`, `time.time` and `datetime.now` reads with the arena clock, by assigning attributes on the imported modules (no edits to the factory folder). It steps `tick` in fixed 100 ms increments through `arena_part.clock.FixedStep`.
- `uses_clock: true`. Restore: exact save plus replay.
- Save = JSON of every Plant and Market attribute plus `rng.getstate()`, made JSON-safe. It must restore to an equal `snapshot()` and equal later behaviour.
- Tools: the `ridgeline-floor` and `ridgeline-market` tool names and arguments from `../sculptured-factory/mcp/src/ridgeline_mcp/{floor,market}.py`, calling Plant and Market methods directly. Keys: `machine/<id>`, `order/<id>`, `technician/<id>`, `shipment/<id>`, `rfq/<id>`, `freight/<id>`, `claim/<id>`.
- `advance` events: plant events with kind `alarm` and similar, from `Plant.emit`.

- [ ] **Step 1:** Write failing tests:
  - `test_scenario_alarm_on_arena_clock`: start the scenario, advance to 70 000 ms, and an `alarm` event is reported.
  - `test_save_restore_equal_future`: save at 30 s; run A to 90 s; restore at 30 s and run B to 90 s; the snapshots are equal.
  - `test_replay_equals_save`.
  - Conformance check passes (via `arena-part-check`, from pytest through subprocess, built binary).
- [ ] **Step 2:** `cd integrations/ridgeline-plant && uv run --with pytest pytest -q` fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** It passes.
- [ ] **Step 5:** Commit "integrations: ridgeline plant on the arena clock".

### Task 14: `sculptured` integration

**Files:**
- Create: `integrations/sculptured/` (binary `arena-sculptured`)
- Test: `integrations/sculptured/tests/live_sculptured.rs` (ignored unless `config.binary` exists; default `~/MLSys/Sculptured/sculptured/target/release/sculptured`)

**Interfaces:**
- Instance = `sculptured serve --port <free loopback port> --data-dir <saves>/sculptured/live/<instance>`. It must never use port 4410 or the user's data dir.
- Save = SQLite online backup of `controld.sqlite` (`sqlite3` CLI `.backup`, or rusqlite feature `backup`; my call: rusqlite, bundled) plus `copy_save` of the rest of the folder.
- Restore = copy into the new data dir, then start. Exact save.
- Tools `propose_change`, `list_changes`, `decide_change`, through Sculptured's HTTP API. The builder looks up the exact routes in `../sculptured/crates/controld` (team changes endpoints) and documents them in the package README.

- [ ] **Step 1:** Write `live_sculptured_roundtrip` (propose, save, decide, restore, the change is undecided again) and run the conformance check.
- [ ] **Step 2:** Run it; it fails.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the live test once for real. Stop every server it started.
- [ ] **Step 5:** Commit "integrations: sculptured approvals".

### Task 15: `isaac-sim` integration (Python, unverified on hardware)

**Files:**
- Create: `integrations/isaac-sim/{arena-package.toml,pyproject.toml,isaac_part/part.py,isaac_part/backend.py,README.md}`
- Test: `integrations/isaac-sim/tests/test_part.py`, with a stand-in backend

**Interfaces:**
- The manifest declares `needs = { gpu_vram_gb = 16 }`.
- `backend.IsaacBackend`:
  - starts `SimulationApp` (headless), opens `config.usd`, and wraps an Isaac Lab `InteractiveScene`;
  - `get_state() -> dict`, `reset_to(state)`, `set_joint_targets(articulation, targets)`, `step(n)`, `fingerprint() -> {gpu, driver, isaac_sim}`.
- `backend.StandIn` has the same interface: a deterministic toy physics in pure Python for tests.
- Part: restore approximate (`get_state` saved as JSON tensors-to-lists) plus replay; `uses_clock: true` (physics steps at `config.physics_hz`, default 60, from the arena clock).
- Tools `get_scene_state` (read), `set_joint_targets` (write), `step` (write).
- `config.mode = "mirror"`: no tools, and the scene follows a plant URL like `../sculptured-factory/isaac/bridge.py` does.
- The README says plainly that it has not run on Isaac Sim here, and why.

- [ ] **Step 1:** Write tests with `StandIn`:
  - the conformance check passes (approximate is reported, not failed; replay is exact);
  - `test_fingerprint_recorded`;
  - `test_mirror_mode_has_no_tools`.
- [ ] **Step 2:** Run them; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** They pass.
- [ ] **Step 5:** Commit "integrations: isaac-sim (unverified on hardware)".

### Task 16: Both UIs: restore choice, labels, saves

**Files:**
- Modify: `arena/ui/src/{types.ts,run/ForkPanel.tsx,components.tsx,pages/Run*.tsx,pages/Compare*.tsx}` and the tests next to them.
- Modify, in `../sculptured-tests` (branch `tests-arena`): `web/src/tests/{types.ts,RerunPanel.tsx,RunPage.tsx,ComparePage.tsx,TestsHome.tsx,StepInspector.tsx}` and their tests.

**Interfaces:**
- Consumes the Task 9 fields: `Branch.restore_mode`, `Branch.restored`, `ToolCallPayload.saves|part|clock_ms`, and `/api/branches/{id}/restore-plan`.
- **Rerun panel:**
  - a "Restore" choice, Exact (default) or Approximate;
  - one line per part from restore-plan, e.g. "retail: from its save"; "plant: replayed from the start, about 40 s"; "isaac: approximate";
  - when exact is refused, the server's message is shown, and the start button is disabled with that reason.
- **Labels:** an "Approximate restore" mark on branch rows, the run header and compare sides. Compare adds one line: "Differences may come from the approximate restore as well as the change."
- **Step inspector:** "Saved after this step" reads `saves` (fallback `snapshot`). For several parts it lists them.

- [ ] **Step 1:** Write failing tests in each UI: `rerun panel shows per-part plan`, `approximate branch is labelled`, `exact refusal disables start with reason`.
- [ ] **Step 2:** Run the vitest suites; they fail.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** In both `npx vitest run`, `npx tsc --noEmit` (Sculptured) and `npm run build` pass. Take real captures of the rerun panel and an approximate-labelled branch from a running arenad, and look at them.
- [ ] **Step 5:** Commit, in each repo.

### Task 17: Docs

**Files:**
- Modify: `arena/README.md`: how a world is made of integrations, the two restore modes, writing an integration (kits, manifest, `arena-part-check`), the list of integrations with their restore modes and what was verified live, and an updated "Not built yet".
- Create: `integrations/README.md` (the "drivers" overview plus a 20-line walkthrough of the counter example), and a short README in each integration.

- [ ] **Step 1:** Write the docs. Every claim is checked against the code or a test run; unverified items are said plainly (Isaac Sim).
- [ ] **Step 2:** Commit "docs: integrations".
