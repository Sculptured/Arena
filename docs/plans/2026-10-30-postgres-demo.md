# Reversible Arena: Postgres Demo Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A working first version of the reversible arena. A triage → resolver team runs on τ²-bench retail tasks against Postgres. Every step is recorded with database snapshots. A user can fork any run at a step with a changed prompt, model or mode, and compare the branches. A separate web UI manages it all. The plan also fixes the Sculptured board's layout of imported groups.

**Architecture:**
- The **core** is a Rust workspace (`arena/core`):
  - `world` is a library for the Postgres retail world: schema, tools, snapshots and score.
  - `arenad` is the service: the recording store, an MCP endpoint, a model gateway, agent drivers, the runner, fork, compare, and an HTTP API with SSE.
- The **UI** (`arena/ui`) is a separate React app that talks to arenad only over HTTP.
- **Snapshots** use `CREATE DATABASE … TEMPLATE`.

**Tech Stack:**
- Rust (nightly 1.99 is installed): axum 0.8, tokio, tokio-postgres (feature `with-serde_json-1`), serde/serde_json, reqwest (rustls, json), uuid, anyhow.
- Postgres 17 (Docker).
- React 19, Vite 8, TypeScript, vitest.

**Spec:** `arena/docs/specs/2026-10-30-postgres-demo-design.md`

## Global Constraints

- **Postgres:** `postgres:17-alpine` on host port **5544**. The URL comes from env `ARENA_PG_URL`, default `postgres://arena:arena@127.0.0.1:5544/postgres`.
- **Ports:** arenad listens on **4510**, the UI dev server on **4520**. The UI proxies `/api` to 4510.
- **Databases:**
  - `arena` holds metadata;
  - `world_base` holds the retail data;
  - `gold_<task>`, `branch_<id>` and `snap_<id>` are copies made with `CREATE DATABASE … TEMPLATE`.
  - The core never holds an open connection to a database while copying it.
- **Retail tables:** `users`, `orders` and `products`, each `(id text primary key, doc jsonb not null)`. `doc` is the τ²-bench object, unchanged.
- **Read and write sets:** strings `"<table>/<id>"`, the same as `tool.py` (`orders/#W123`).
- **Writes:** only the last agent in the chain may call write tools. Other agents get the error `"Write tools are not available to this agent"`.
- **Fork:** going back to step N redoes step N. The fork point moves back only by the spec's rules, and the response always gives `moved_from` and `reason`.
- **Cost labels:**
  - `"openrouter"` is the reported cost;
  - `"api_equivalent_estimate"` is Claude Code's `total_cost_usd`;
  - Codex gets `null` dollars, tokens only.
- **Data:** public τ²-bench data only, copied with attribution. No API keys in code or logs. Open mode reads `OPENROUTER_API_KEY` from env, and without it the mode is reported as unavailable.
- **Demo tasks:** the 10 seed-2680 tasks `27, 72, 81, 86, 88, 91, 93, 94, 96, 102` (from `cs2680-gap-tests/results/tasks.json`).
- **Default open model:** `deepseek/deepseek-chat-v3.1`.
- **Commits:** none unless the user has said to commit. Where a step says "Checkpoint", run the full test command.

## Review Focus

1. **A fork or a new run while the source branch is still running.** Expected: 409 with "the branch is still running". The test goes in Task 10.
2. **Two runs at once.** Snapshot copies of different branches must not fail with "source database is being accessed by other users". The test goes in Task 4 (concurrent `snapshot` on two branches).
3. **A CLI agent dies or hits a usage limit.** Expected: the branch ends `infra_error`, it isn't scored as a failure, and the error text is stored. The test goes in Task 8 (a parse of an `is_error: true` output) and Task 9 (a runner with a driver that errors).
4. **arenad restarts mid-run.** Expected: branches left `running` are marked `interrupted` at startup. The test goes in Task 5.
5. **Bad tool calls.** An unknown tool, malformed arguments or a missing field should come back as a tool error result that gets recorded, never an HTTP 500. The test goes in Task 6.

---

## Part A: Sculptured board fix

### Task 1: Fit imported groups around their cards

**Files:**
- Modify: `sculptured/web/src/draft/edits.ts` (add an exported helper next to `fitGroups`, about line 120)
- Modify: `sculptured/web/src/useTeamDraft.ts:130-139` (the load path)
- Test: `sculptured/web/src/draft/edits.test.ts`
- Modify: `sculptured-factory/team/build_team.py` (group sizes)

**Interfaces:**
- Produces: `export function groupsFit(d: TeamDraft): boolean`. It returns true when every agent's card box (`CARD_W`×`CARD_H` at `a.x, a.y`) lies inside its group with `GROUP_PAD` on all sides and `GROUP_HEAD` on top, and no two drawn groups overlap.

- [ ] **Step 1: Write the failing tests** in `edits.test.ts`. Load `../../../sculptured-factory/team/ridgeline-incident.json` with `readFileSync`; if the file is missing, inline its `groups` and `agents` (the values are listed in the spec's bug section).
  - `groupsFit(ridgeline.team)` is `false`.
  - `groupsFit(fitGroups(ridgeline.team))` is `true`.
  - `fitGroups(fitGroups(x))` deep-equals `fitGroups(x)`, so fitting is idempotent and a fitted team never re-saves.
- [ ] **Step 2:** Run `cd sculptured/web && npx vitest run src/draft/edits.test.ts`. Expected: FAIL, `groupsFit` not exported.
- [ ] **Step 3: Implement `groupsFit`.** In `useTeamDraft`'s load `.then`, when `!groupsFit(rec.draft)`, start the history from `fitGroups(rec.draft)` instead of `rec.draft`. `sent.current` stays the server draft, so the normal autosave saves the fitted layout once. Leave the history's undo stack empty.
- [ ] **Step 4:** Run `npx vitest run` and `npm run build`. Expected: all pass.
- [ ] **Step 5: Check by eye.** Re-run `sculptured-factory/scripts/install_team.py` (Sculptured on 4410). Open `http://127.0.0.1:4410/p/ridgeline-incident/board` and screenshot it with headless Chrome into the scratchpad. Expected: every card is inside its group and no header sits under a card.
- [ ] **Step 6: Fix `build_team.py`.**
  - Each group's height is `GROUP_HEAD + 2*GROUP_PAD + CARD_H` plus 16 of slack, which is 292.
  - The gap between group rows is at least 16.
  - The Incident group is as wide as its one card plus padding.
  - Rebuild the JSON and assert `groupsFit` with a one-off node or vitest check.

---

## Part B: the arena

### Task 2: Repository, Postgres, data, and the retail world loader

**Files:**
- Create: `arena/README.md` (how to run; τ²-bench attribution, MIT)
- Create: `arena/.gitignore` (`core/target`, `ui/node_modules`, `ui/dist`)
- Create: `arena/deploy/compose.yml` (service `pg`, `postgres:17-alpine`, `POSTGRES_USER/PASSWORD=arena`, port `5544:5432`, named volume)
- Create: `arena/data/retail/{db.json,tasks.json,policy.md}`, copied from `cs2680-gap-tests/vendor/tau2-bench/data/tau2/domains/retail/`
- Create: `arena/data/retail/demo_tasks.json` (`["27","72","81","86","88","91","93","94","96","102"]`)
- Create: `arena/core/Cargo.toml` (workspace with `crates/world` and `crates/arenad`)
- Create: `arena/core/crates/world/src/{lib.rs,pg.rs,schema.rs}`
- Test: `arena/core/crates/world/tests/load.rs`

**Interfaces:**
- Produces:
  - `world::pg::Admin::connect(url: &str) -> Result<Admin>`. It connects to the `postgres` database.
  - `Admin::db_url(&self, db: &str) -> String`.
  - `Admin::create_db_from(&self, new: &str, template: &str) -> Result<()>`.
  - `Admin::drop_db(&self, name: &str) -> Result<()>`.
  - `Admin::exists(&self, name: &str) -> Result<bool>`.
  - `world::schema::load_base(admin: &Admin, data_dir: &Path) -> Result<()>`. It creates `world_base` if it's missing and loads `db.json`. It is idempotent.
  - `world::dump(client: &tokio_postgres::Client) -> Result<serde_json::Value>`. It returns the canonical `{"users":{id:doc},"orders":{…},"products":{…}}`.
  - The test helper `world::testing::admin() -> Admin` reads `ARENA_PG_URL`.

- [ ] **Step 1:** Run `docker compose -f arena/deploy/compose.yml up -d`, then `pg_isready -h 127.0.0.1 -p 5544` (or `docker exec` it). Expected: accepting connections.
- [ ] **Step 2: Write the failing test** `load.rs::base_round_trips`. It runs `load_base` twice, then checks:
  - `dump(world_base)` equals `serde_json::from_str(db.json)`;
  - the counts are 500 users, 1000 orders and 50 products.
- [ ] **Step 3:** Run `cd arena/core && cargo test -p world --test load`. Expected: FAIL (it doesn't compile).
- [ ] **Step 4: Implement.** Use one `INSERT … SELECT * FROM jsonb_each($1)` per table, or batched inserts.
- [ ] **Step 5:** Run the test. Expected: PASS.
- [ ] **Step 6:** Run `git init` in `arena/`. Checkpoint.

### Task 3: Retail tools on Postgres, with read and write sets

**Files:**
- Create: `arena/core/crates/world/src/tools.rs` (a port of `cs2680-gap-tests/harness/tool.py`, class `Retail` and `call`)
- Create: `arena/core/crates/world/tests/fixtures/make_gold.py` and its output `gold_changes.json`
- Test: `arena/core/crates/world/tests/tools.rs`

**Interfaces:**
- Produces:
  - `pub const READ_TOOLS: &[&str]` and `pub const WRITE_TOOLS: &[&str]`, the same names as `tool.py`.
  - `pub struct ToolOutcome { pub result: Option<Value>, pub error: Option<String>, pub reads: Vec<String>, pub writes: Vec<String> }`, with sorted sets.
  - `pub async fn call(client: &mut Client, name: &str, args: &Value) -> Result<ToolOutcome>`. It runs in one transaction. An `Err` is only for database failures. Tool failures, unknown tools and bad arguments come back as `ToolOutcome.error`.
  - `pub fn tool_schemas() -> Vec<Value>`. These are MCP-style `{name, description, inputSchema}` entries, with the argument names from `gap.py`'s `TOOL_DOC`.
  - `world::gold::ensure_gold(admin, task: &Value) -> Result<String>`. It creates `gold_<task_id>` from `world_base`, applies the gold actions, and returns the database name. It is cached. A failing gold action leaves the database unchanged, as upstream does.

- [ ] **Step 1: Write `make_gold.py`.**
  - It imports `tool` from `cs2680-gap-tests/harness`.
  - For every eligible task, it applies the gold actions as `gap.gold_db` does. A task is eligible when it has a write action and no `nl_assertions`; there are 68.
  - It writes `{task_id: {"users/<id>": doc, "orders/<id>": doc, …}}` with only the rows that differ from base.
  - Run it with `python3 -I`. Expected: 68 keys, file under 1 MB.
- [ ] **Step 2: Write the failing tests:**
  - `gold_changes_match_for_all_68_tasks`: for each task, copy `world_base` into a temp database, `call` each gold action, `dump`, and check that the rows that differ from base equal the fixture exactly.
  - `reads_and_writes_are_recorded`: `get_order_details {"order_id":"#W2378156"}` has reads `["orders/#W2378156"]` and no writes.
  - `unknown_tool_and_bad_args_are_tool_errors`: `call(.., "nope", {})` gives `error == "Unknown tool nope"`, and a missing argument gives `error.is_some()`, not `Err`.
- [ ] **Step 3:** Run `cargo test -p world --test tools`. Expected: FAIL.
- [ ] **Step 4: Implement.** Load each touched row's `doc` with `SELECT doc … FOR UPDATE`, mutate it as `serde_json::Value` following `tool.py` line by line (including `fold` for names), and `UPDATE` the changed rows. Floats round the same way `tool.py` does: `round(x, 2)`.
- [ ] **Step 5:** Run the tests. Expected: PASS for all three. Checkpoint.

### Task 4: Branches and snapshots

**Files:**
- Create: `arena/core/crates/world/src/snapshot.rs`
- Test: `arena/core/crates/world/tests/snapshot.rs`

**Interfaces:**
- Produces: `pub struct Worlds { admin: Admin, locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>> }`. There is one lock per database name, held during any copy *from* or connection *to* that database.
  - `Worlds::new_branch(&self, id: &str, from: Source) -> Result<String>`. `Source` is `Base`, `Snapshot(db_name)` or `Gold(task)`. It returns `branch_<id>`.
  - `Worlds::snapshot(&self, branch_db: &str) -> Result<String>`. It returns `snap_<uuid>`.
  - `Worlds::with_client<F, T>(&self, db: &str, f: F) -> Result<T>`. It holds the database's lock, connects, runs `f`, and closes the connection.
  - `Worlds::drop(&self, db: &str)`.

- [ ] **Step 1: Write the failing tests:**
  - `snapshot_then_restore_round_trips`: make a branch, write with `call`, snapshot, write again, then make a new branch from the snapshot. Its `dump` equals the dump taken right after the first write.
  - `concurrent_snapshots_of_two_branches_succeed`: `tokio::join!` 10 snapshot-and-write cycles on two branches. There are no errors.
  - `snapshot_time_is_reported`: print the median copy time of 10 snapshots, for the README. Assert nothing about its value.
- [ ] **Step 2:** Run `cargo test -p world --test snapshot`. Expected: FAIL.
- [ ] **Step 3: Implement.** `CREATE DATABASE` can't take parameters, so names are generated as `[a-z0-9_]` only and checked before use. Postgres also refuses a copy while any session is connected to the source, so the per-database lock serializes copies against `with_client`.
- [ ] **Step 4:** Run the tests. Expected: PASS. Checkpoint.

### Task 5: Recording store

**Files:**
- Create: `arena/core/crates/arenad/src/{main.rs,store.rs,model.rs}`
- Test: `arena/core/crates/arenad/tests/store.rs`

**Interfaces:**
- Produces, in `model.rs` (serde, `snake_case`):
  - `AgentKind { ClaudeCode, Codex, Open }`.
  - `AgentSpec { id: String, name: String, kind: AgentKind, model: Option<String>, prompt: String }`.
  - `Team { id: String, name: String, agents: Vec<AgentSpec> }`. This is a chain; the last agent writes.
  - `Change { prompt: Option<String>, model: Option<String>, kind: Option<AgentKind> }`.
  - `BranchStatus { Running, Passed, Failed, InfraError, Interrupted }`.
  - `Event { branch_id: String, seq: i64, at: DateTime, agent: Option<String>, kind: EventKind, payload: Value }`.
  - `EventKind { RunStart, AgentStart, ModelCall, ToolCall, Handoff, AgentEnd, Check }`.
  - Payloads:
    - `ToolCall`: `{name, args, result, error, reads, writes, snapshot: Option<String>, tool_call_id: Option<String>}`.
    - `ModelCall`: `{request, response, usage, cost_usd, cost_source}`.
    - `AgentEnd`: `{ok, error, tokens, cost_usd, cost_source, seconds}`.
    - `Handoff`: `{note}`.
    - `Check`: `{passed, gold_db, differing_rows}`.
- Produces, in `store.rs`, `Store`:
  - `Store::open(admin: &Admin) -> Result<Store>`. It creates the `arena` database and its tables if they're missing, then marks every `running` branch `interrupted`.
  - `create_run(task_id, team) -> RunId` and `create_branch(run_id, parent: Option<(branch_id, start_seq)>, changes, fork_note: Option<String>) -> BranchId`.
  - `append(branch_id, agent, kind, payload) -> Event`. It assigns the next `seq` and publishes the event on a `tokio::sync::broadcast` channel.
  - `events(branch_id) -> Vec<Event>`, `set_status(branch_id, status, error: Option<String>)`, `runs() -> Vec<RunSummary>` and `branch(id) -> Branch`.
  - `subscribe() -> broadcast::Receiver<Event>`.
  - `copy_prefix(from_branch, to_branch, before_seq)`. It copies the parent's events with `seq < before_seq`, keeping their `seq`s.
- Tables: `runs`, `branches` (with `parent_id`, `start_seq`, `changes jsonb`, `fork_note`, `status`, `error`, timestamps) and `events` (primary key `(branch_id, seq)`).

- [ ] **Step 1: Write the failing tests:**
  - `append_assigns_increasing_seq`.
  - `copy_prefix_keeps_seqs_and_new_events_continue_after`.
  - `open_marks_running_branches_interrupted`, which covers Review Focus 4.
- [ ] **Step 2:** Run `cargo test -p arenad --test store`. Expected: FAIL.
- [ ] **Step 3: Implement.** Each test uses a fresh metadata database, named by a test-only `Store::open_named`.
- [ ] **Step 4:** Run the tests. Expected: PASS. Checkpoint.

### Task 6: The MCP endpoint (tool calls, handoff, recording, snapshots)

**Files:**
- Create: `arena/core/crates/arenad/src/{mcp.rs,exec.rs,app.rs}`
- Test: `arena/core/crates/arenad/tests/mcp.rs`

**Interfaces:**
- Consumes: `world::tools::{call, tool_schemas}`, `Worlds` and `Store`.
- Produces:
  - `exec::ToolExec::run(&self, branch: &BranchCtx, agent: &str, name: &str, args: Value, tool_call_id: Option<String>) -> Value`. It returns the result or error JSON that is given back to the agent. It refuses writes when the agent isn't last. `handoff {note}` is allowed only for agents that aren't last; it records a `Handoff` event and marks the agent's handoff as done. After a write succeeds, it calls `Worlds::snapshot` and sets `snapshot` on the `ToolCall` event. Every call appends exactly one `ToolCall` event, or a `Handoff` event.
  - `BranchCtx { branch_id, db: String, team: Team, task_id }`. Live branches are kept in an `AppState.live: RwLock<HashMap<branch_id, BranchCtx>>`.
  - `mcp::router()` handles `POST /mcp/{branch}/{agent}`. It speaks MCP streamable HTTP with plain JSON responses, handling `initialize` (protocol version `2025-06-18`, capabilities `{tools:{}}`, server name `arena-retail`), `notifications/initialized` (answered 202 with no body), `tools/list` (the retail tools plus `handoff` when allowed) and `tools/call`. Tool errors come back as `{content:[{type:"text",text}], isError:true}`.
  - `app::AppState { store, worlds, live, config }` and `app::router(state) -> axum::Router`.

- [ ] **Step 1: Write the failing tests.** Start arenad's router on a random port with a 2-agent team and drive it with reqwest JSON-RPC:
  - `tools_list_has_handoff_only_for_first_agent`.
  - `triage_write_is_refused_and_recorded`.
  - `resolver_write_snapshots`: the `ToolCall` event has `snapshot: Some`, and a branch made from that snapshot shows the write.
  - `bad_calls_are_tool_errors_not_500`, covering Review Focus 5: an unknown tool, `arguments: "x"`, and missing fields all give HTTP 200 with `isError: true`, and each gives one event.
- [ ] **Step 2:** Run `cargo test -p arenad --test mcp`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests. Expected: PASS. Checkpoint.

### Task 7: Model gateway and the open agent loop

**Files:**
- Create: `arena/core/crates/arenad/src/{gateway.rs,agents/mod.rs,agents/open.rs}`
- Test: `arena/core/crates/arenad/tests/open.rs` and `tests/support/fake_upstream.rs`

**Interfaces:**
- Consumes: `ToolExec` (in process, not over HTTP).
- Produces:
  - `gateway::router()` handles `POST /gateway/{branch}/{agent}/v1/chat/completions`. It forwards to `config.upstream_base` (default `https://openrouter.ai/api/v1`) with `Authorization: Bearer $OPENROUTER_API_KEY` and adds `"usage":{"include":true}`. It appends a `ModelCall` event with the request, response, `usage`, and `cost_usd` from `usage.cost` with `cost_source: "openrouter"`. On an upstream error it returns 502 and records the event with the error.
  - `agents::Driver` trait: `async fn run(&self, ctx: AgentRun) -> AgentResult`.
  - `AgentRun { branch: BranchCtx, spec: AgentSpec, input: String, resume: Option<Vec<Value>> }`. `input` is the first user message. `resume` is a replayed message list.
  - `AgentResult { ok: bool, error: Option<String>, tokens: Value, cost_usd: Option<f64>, cost_source: Option<String>, seconds: f64 }`.
  - `agents::open::OpenDriver`. Its loop:
    1. Send messages (system = `spec.prompt`, then `input`, or `resume` with its system message replaced by `spec.prompt`) and the tools (OpenAI `tools` from `tool_schemas()`, plus `handoff` when allowed) to the gateway URL for this branch and agent.
    2. Run each `tool_calls` entry through `ToolExec` and append `{role:"tool", tool_call_id, content}`.
    3. Stop when there are no tool calls, or after the agent's handoff, or after 40 model calls (an error).
  - `config.open_available()` is `OPENROUTER_API_KEY.is_some() || upstream_base is not openrouter`.
  - `tests/support/fake_upstream.rs` serves scripted responses in order: a `Vec<Value>` of assistant messages, with fixed `usage`.

- [ ] **Step 1: Write the failing tests:**
  - `open_agent_runs_scripted_tools_and_records_model_calls`: the scripted script is `get_order_details`, then the gold write, then a final text. Expect 3 `ModelCall` and 2 `ToolCall` events, in order.
  - `upstream_error_is_recorded_and_agent_fails`.
  - `forty_call_cap_stops_the_loop`.
- [ ] **Step 2:** Run `cargo test -p arenad --test open`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests. Expected: PASS. Checkpoint.

### Task 8: CLI drivers (Claude Code and Codex)

**Files:**
- Create: `arena/core/crates/arenad/src/agents/{claude.rs,codex.rs}`
- Create: `arena/core/crates/arenad/tests/fixtures/{claude_ok.json,claude_error.json,codex_ok.jsonl}` (trimmed from real outputs in `cs2680-gap-tests/runs/`)
- Test: `arena/core/crates/arenad/tests/cli.rs`

**Interfaces:**
- Produces:
  - `claude::command(spec, mcp_url, prompt) -> std::process::Command` and `claude::parse(stdout) -> AgentResult` (fields only, with `seconds` set by the caller). `cost_source` is `"api_equivalent_estimate"`. With `is_error: true`, `ok` is false and `error` is the `result` text.
  - `codex::command(spec, mcp_url, workdir, prompt) -> Command` and `codex::parse(stdout_jsonl, returncode) -> AgentResult`. Tokens come from `turn.completed.usage`, and `cost_usd` is `None`.
  - `ClaudeDriver` and `CodexDriver` implement `Driver`. Each runs in a fresh empty temp directory with a 900 s timeout.
- The command lines (start points; Step 5 confirms them live):
  - **claude:** `claude -p <prompt> --model <model or sonnet> --output-format json --setting-sources "" --strict-mcp-config --mcp-config '{"mcpServers":{"retail":{"type":"http","url":"<mcp_url>"}}}' --disable-slash-commands --tools "" --allowedTools "mcp__retail__*"`.
  - **codex:** `codex exec --skip-git-repo-check --json -s read-only -C <workdir> -c 'mcp_servers.retail.url="<mcp_url>"' -c 'approval_policy="never"' [-m <model>] <prompt>`.
  - The prompt is the agent's `prompt`, then `input` (the ticket, the policy and the handoff note).

- [ ] **Step 1: Write the failing tests:**
  - `claude_command_has_only_retail_mcp`: the args contain `--strict-mcp-config` and the given URL.
  - `claude_parse_ok_and_error`, covering Review Focus 3.
  - `codex_parse_tokens`.
  - `codex_command_sets_mcp_url`.
- [ ] **Step 2:** Run `cargo test -p arenad --test cli`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests. Expected: PASS.
- [ ] **Step 5: Live smoke tests** (`#[ignore]`; run with `cargo test -p arenad --test cli -- --ignored live_`). They use the user's sign-in and cost a few cents.
  - `live_claude_lists_tools`: a one-line prompt asks the agent to call `list_all_product_types` once. Expect one `ToolCall` event.
  - `live_codex_lists_tools`: the same for Codex.
  - If a flag is wrong, fix the command and record the working flags in a comment. Checkpoint.

### Task 9: Runner and score

**Files:**
- Create: `arena/core/crates/arenad/src/{runner.rs,tasks.rs,prompts.rs}`
- Test: `arena/core/crates/arenad/tests/runner.rs`

**Interfaces:**
- Consumes: `Driver`s, `Store`, `Worlds`, `world::gold::ensure_gold` and `world::dump`.
- Produces:
  - `tasks::Tasks::load(data_dir) -> Tasks`, with `demo() -> Vec<TaskSummary{id, reason}>`, `get(id) -> Value` and `ticket(id) -> String`. The ticket text is `gap.py::ticket` verbatim.
  - `prompts::default_team() -> Team`. Agents `triage` (ClaudeCode, sonnet) and `resolver` (ClaudeCode, sonnet). The prompts are `gap.py`'s `TRIAGE_PROMPT` and `RESOLVER_PROMPT`, reworded to "call the tools" and "call handoff with your note" instead of files and `tool.py`.
  - `runner::start_run(state, task_id, team) -> (RunId, BranchId)` creates a branch from `world_base` and spawns the work.
  - `runner::run_branch(state, branch_ctx, from: StartPoint)`. `StartPoint { agent_index, resume: Option<Vec<Value>>, notes: Vec<String> }`. It runs agents from `agent_index` in order; each agent's input is the ticket, the policy, and the previous agent's note. Then it scores and sets the status:
    - `Check` passes when `dump(branch) == dump(gold)`, and `differing_rows` lists the keys that differ;
    - status is `passed` or `failed` by the check;
    - status is `infra_error` when any agent returns `ok: false` with an error, in which case there is no `Check` event;
    - status is `failed` with the error "no handoff" when a non-last agent ends without a handoff.
  - `AppState.drivers: HashMap<AgentKind, Arc<dyn Driver>>`, so tests can swap in a scripted driver.

- [ ] **Step 1: Write the failing tests.** Both agents are open, using the fake upstream.
  - `scripted_team_passes_task_27`: the scripts are the triage reads plus `handoff`, then the resolver's gold actions. Status is `passed`, and the event order is `RunStart, AgentStart(triage) … Handoff, AgentEnd, AgentStart(resolver) … AgentEnd, Check`.
  - `wrong_write_fails_with_differing_rows`.
  - `driver_error_is_infra_error_not_failed`, covering Review Focus 3.
  - `missing_handoff_fails`.
- [ ] **Step 2:** Run `cargo test -p arenad --test runner`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests. Expected: PASS. Checkpoint.

### Task 10: Fork

**Files:**
- Create: `arena/core/crates/arenad/src/fork.rs`
- Test: `arena/core/crates/arenad/tests/fork.rs` (with unit tests of `plan_fork` in-module)

**Interfaces:**
- Produces:
  - `fork::plan_fork(events: &[Event], team: &Team, at_seq: i64, changes: &HashMap<String, Change>) -> Result<ForkPlan, ForkError>`.
  - `ForkPlan { start_seq: i64, moved_from: Option<i64>, reason: Option<String>, agent_index: usize, restore: Restore, resume: Option<Vec<Value>>, notes: Vec<String> }`, where `Restore` is `Base` or `Snapshot(String)`. The algorithm:
    1. Let `a` be the agent whose `AgentStart` is the last one at or before `at_seq`. If `at_seq` is the `RunStart`, `a` is the first agent.
    2. If any changed agent comes before `a` in the chain, set `a` to the earliest changed agent and `start_seq` to its `AgentStart` seq, with the reason "a change targets an agent that finished earlier".
    3. Otherwise, if `a` is a CLI agent, or `changes[a].kind` is set, `start_seq` is `a`'s `AgentStart`, with the reason "Claude Code and Codex sessions restart from the agent's start", or "the agent's mode changes".
    4. Otherwise `a` is open. If `at_seq` is a `ToolCall`, `start_seq` is the `ModelCall` that issued it, with the reason "a tool call is redone from the model call that chose it". If it is a `ModelCall`, `start_seq` is `at_seq`. `resume` is that `ModelCall`'s `request.messages`. If `start_seq` is `a`'s `AgentStart`, `resume` is `None`.
    5. `restore` is the last `ToolCall.snapshot` with `seq < start_seq`, or `Base`.
    6. `notes` holds the `Handoff` notes of agents before `a`.
    7. `moved_from` is `Some(at_seq)` whenever `start_seq != at_seq`.
  - `ForkError`: `UnknownSeq`, `UnknownAgent(String)` and `BranchRunning`.
  - `fork::fork(state, branch_id, at_seq, changes) -> Result<ForkResponse{branch_id, start_seq, moved_from, reason}>`. It plans the fork, creates the branch (`parent`, `start_seq`), runs `copy_prefix`, applies the changes to a copy of the team, calls `Worlds::new_branch` with `Source::Base` or `Source::Snapshot` to match `restore`, and spawns `run_branch` at the plan's start point.

- [ ] **Step 1: Write the failing unit tests of `plan_fork`,** with hand-built event lists:
  - `cli_agent_moves_to_agent_start`;
  - `open_tool_call_moves_to_its_model_call`;
  - `open_model_call_stays_and_resumes_messages`;
  - `change_to_finished_agent_moves_back`;
  - `kind_change_moves_to_agent_start`;
  - `restore_is_last_snapshot_before_start`.
- [ ] **Step 2: Write the failing integration tests** with the fake upstream:
  - `fork_sees_world_at_fork_step_not_after`: after a run whose resolver wrote, fork at the resolver's write `ModelCall`. The new branch's first resolver read shows the order as it was before the write. This is the gap-test failure that the platform fixes.
  - `fork_reuses_recorded_handoff`: the triage driver isn't called again.
  - `fork_applies_prompt_change`: the fake upstream records that the system message equals the new prompt.
  - `fork_while_running_is_409`, covering Review Focus 1.
- [ ] **Step 3:** Run `cargo test -p arenad --test fork` and `cargo test -p arenad --lib fork`. Expected: FAIL.
- [ ] **Step 4:** Implement.
- [ ] **Step 5:** Run the tests. Expected: PASS. Checkpoint.

### Task 11: HTTP API, SSE, compare, and the binary

**Files:**
- Create: `arena/core/crates/arenad/src/{api.rs,compare.rs}`
- Modify: `arena/core/crates/arenad/src/{main.rs,app.rs}`
- Test: `arena/core/crates/arenad/tests/api.rs`

**Interfaces:**
- Produces these routes, all JSON:
  - `GET /api/health` returns `{open_available, claude: bool, codex: bool}`, where the CLIs are found on `PATH`.
  - `GET /api/tasks` returns the demo tasks. `GET /api/team/default` returns the default team.
  - `POST /api/runs {task_id, team}` returns `{run_id, branch_id}`. `GET /api/runs` returns runs with their branches (status, parent, start_seq, fork_note).
  - `GET /api/branches/{id}` returns the branch plus `events`. `GET /api/branches/{id}/stream` is SSE: one `event` per new `Event` of that branch, then `status` at the end.
  - `GET /api/branches/{id}/rows?seq=N&keys=orders/%23W1,users/u1` returns those rows as they were just after step N. They come from a temp branch restored from the last snapshot at or before N, which is dropped afterwards.
  - `POST /api/branches/{id}/fork {at_seq, changes}` returns `ForkResponse`, or 409/400.
  - `GET /api/compare?a=&b=` returns `{a:{status, seconds, tokens, cost_usd, cost_source}, b:{…}, first_divergent_seq, differing_rows:[{key, a, b}]}`.
    - `first_divergent_seq` is the first seq at which the two event lists differ in `(agent, kind, name, args)`.
    - `differing_rows` compares the two final databases; the dumps are cached per finished branch.
  - `main.rs` reads `ARENA_PG_URL`, `ARENA_DATA` (default `../data`), `ARENA_PORT` (default 4510), `OPENROUTER_API_KEY` and `ARENA_UI_DIST`. When `ARENA_UI_DIST` is set, it serves the built UI at `/`.

- [ ] **Step 1: Write the failing tests:**
  - `post_run_then_stream_until_status`;
  - `compare_reports_first_divergence_and_rows` (two scripted runs that differ in one write);
  - `rows_at_seq_shows_state_after_that_step`.
- [ ] **Step 2:** Run `cargo test -p arenad --test api`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run the tests, then `cargo test` for the whole workspace. Expected: all PASS.
- [ ] **Step 5:** Run `cargo run -p arenad`, then `curl -s localhost:4510/api/health`. Expected: JSON with `claude: true` on this machine. Checkpoint.

### Task 12: UI shell, the Runs page and the Team page

**Files:**
- Create: `arena/ui/{package.json,vite.config.ts,tsconfig.json,index.html}` (Vite proxy `/api` → `http://127.0.0.1:4510`, port 4520)
- Create: `arena/ui/src/{main.tsx,App.tsx,api.ts,types.ts}`
- Create: `arena/ui/src/theme/tokens.css` (copied from `sculptured/web/src/tokens.css`, keeping its header comment and adding a "copied from Sculptured" line), plus any fonts it references, copied from `sculptured/web/public`
- Create: `arena/ui/src/app.css` (shell layout following Sculptured's `.app`: a 48 px top bar, then a header with tabs, then the content)
- Create: `arena/ui/src/pages/{Runs.tsx,Team.tsx}`
- Test: `arena/ui/src/api.test.ts`

**Interfaces:**
- Consumes: Task 11's routes.
- Produces:
  - `types.ts` mirrors the Rust `model.rs` names: `Team`, `AgentSpec`, `AgentKind = 'claude_code'|'codex'|'open'`, `Change`, `Event`, `EventKind`, `Branch` and `ForkResponse`.
  - `api.ts` has `health()`, `tasks()`, `defaultTeam()`, `startRun(task_id, team)`, `runs()`, `branch(id)`, `stream(id, onEvent, onStatus): () => void` (an EventSource), `rows(id, seq, keys)`, `fork(id, at_seq, changes)` and `compare(a, b)`.
  - Routes, using `react-router-dom@7`: `/` (Runs), `/team`, `/runs/:runId/:branchId` and `/compare/:a/:b`.
  - **Runs page:** a list of runs, each with its branch tree (indented by parent), status chips (passed, failed, infra error, running, interrupted) and a "New run" form (task select from `tasks()`; it uses the team from the Team page).
  - **Team page:** the chain of agents as cards. Each card has a name, a mode select (Open is disabled with "needs OPENROUTER_API_KEY" when `health().open_available` is false), a model and a prompt textarea, plus add, remove and move-up controls. The team is kept in `localStorage`, wrapped in try/catch, and falls back to `defaultTeam()`.

- [ ] **Step 1: Write the failing test** `api.test.ts::stream_parses_events`, with a mocked EventSource.
- [ ] **Step 2:** Run `cd arena/ui && npm install && npx vitest run`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npx vitest run && npm run build`. Expected: PASS, with no type errors.
- [ ] **Step 5:** Run arenad and `npm run dev`, then screenshot `/` and `/team` with headless Chrome into the scratchpad and check them by eye: Sculptured's dark look, readable, no overflow at 1280 px. Checkpoint.

### Task 13: The Run page and the Compare page

**Files:**
- Create: `arena/ui/src/pages/{Run.tsx,Compare.tsx}`
- Create: `arena/ui/src/run/{timeline.ts,Timeline.tsx,Inspector.tsx,ForkPanel.tsx}`
- Test: `arena/ui/src/run/timeline.test.ts`

**Interfaces:**
- Produces:
  - `timeline.ts`: `lanes(events: Event[], team: Team): Lane[]`, with one lane per agent. `Lane { agent, steps: Step[] }` and `Step { seq, kind, label, wrote: boolean, snapshot: boolean, inherited: boolean }`. `inherited` means the step's seq is below the branch's `start_seq`.
  - **Run page:**
    - a header with the task id, status, seconds, and tokens and cost with its source label;
    - the timeline, live through `stream`;
    - the Inspector for the selected step: arguments, the result or error, reads and writes, and a "Rows at this step" button that calls `rows(seq, writes ∪ reads)`; for a model call it shows messages, the response and usage;
    - the ForkPanel, "Change and rerun from here": a change editor per agent (prompt, model, mode) that calls `fork`. It navigates to the new branch and shows `reason` as a notice when `moved_from` is set;
    - a sidebar with the branch tree and a "Compare with…" action.
  - **Compare page:** two columns of outcome, seconds, tokens and cost (labeled); "First divergence at step N" with both steps; and a table of differing rows showing JSON diffs of the `doc` fields.

- [ ] **Step 1: Write the failing tests** in `timeline.test.ts`:
  - `lanes_group_by_agent_in_chain_order`;
  - `inherited_steps_marked_below_start_seq`;
  - `write_steps_flagged`.
- [ ] **Step 2:** Run `npx vitest run src/run`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `npx vitest run && npm run build`. Expected: PASS.
- [ ] **Step 5: Check end to end by eye.**
  - Start one run on task 27 with the default team (Claude Code).
  - Watch it stream, then open the resolver's write step and check "Rows at this step".
  - Fork at the resolver's first step with a changed prompt, then compare.
  - Screenshot each page into the scratchpad. Checkpoint.

### Task 13b: Disk hygiene (added 2026-10-30 at the user's request: "make sure not to fill up the disk")

Each copy of the retail database is about 21 MB, and every successful write makes a snapshot, so databases add up. Test runs had already left 53 snapshots behind (1.1 GB).

**Files:**
- Modify: `core/crates/world/src/snapshot.rs`, `core/crates/arenad/src/{store.rs,api.rs,main.rs,app.rs}`
- Modify: every test binary that creates databases (`core/crates/world/tests/*.rs`, `core/crates/arenad/tests/*.rs`)
- Modify: `ui/src/pages/Runs.tsx`, `ui/src/pages/Run.tsx`, `ui/src/api.ts`
- Test: `core/crates/arenad/tests/disk.rs`

**Interfaces:**
- Produces:
  - `Worlds::list(prefix: &str) -> Result<Vec<(String, i64)>>`: database names and sizes in bytes.
  - `Store::delete_run(run_id) -> Result<Vec<String>>`: deletes the run's branches and events and returns every database name they referenced (branch databases and the `snapshot` fields of their `ToolCall` events).
  - `DELETE /api/runs/{id}`: 409 if any of its branches is running; otherwise it drops every referenced database (errors are ignored per database but logged) and the metadata, and returns `{dropped: n}`.
  - `GET /api/health` gains `disk: {databases: n, bytes: n, limit_bytes: n}`.
  - Before creating a run or a fork, `POST /api/runs` and `POST …/fork` refuse with 507 "the arena's databases use more than the limit; delete old runs" when the total size of the `branch_%`, `snap_%` and `gold_%` databases is at least `ARENA_MAX_DB_BYTES` (default 5 GiB).
  - At startup, arenad drops `branch_`/`snap_` databases that no event or branch in the store references (orphans from crashes or tests), and logs how many it dropped and how many bytes they used. It never touches `world_base`, `gold_*`, `arena` or databases of other prefixes.
  - UI: a "Delete run" action on the Runs page (with an inline confirm, not `window.confirm`) and the disk line in the top bar.

- [ ] **Step 1: Write the failing tests** in `tests/disk.rs`:
  - `delete_run_drops_its_databases`: run a scripted team that writes once, then delete the run. Its branch database and snapshot no longer exist.
  - `delete_running_run_is_409`.
  - `limit_refuses_new_runs`: with `ARENA_MAX_DB_BYTES` set to 1, POST gives 507.
  - `startup_gc_drops_only_orphans`: create an orphan `snap_` and a referenced one; after the startup GC only the referenced one remains.
- [ ] **Step 2: Make every existing test drop every database it creates** (branch, snapshot, temp copy, metadata store, gold copy), including on assertion failure (use a drop guard). Add a final check to each test binary that the count of `branch_%`/`snap_%`/`arena_test%` databases matches what it was before the binary ran.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run `cargo test` and `cd ui && npx vitest run && npm run build`. Expected: all pass, and afterwards `psql -c "select count(*) from pg_database where datname like 'snap_%'"` is the same as before the tests.
- [ ] **Step 5:** Checkpoint.

### Task 14: Live runs and the README

**Files:**
- Modify: `arena/README.md`

- [ ] **Step 1: Run live through the API:**
  - task 27 with the default team (Claude Code → Claude Code);
  - the same task with the resolver switched to Codex;
  - one fork of the first run at the resolver's start, with the resolver switched to Codex;
  - with an `OPENROUTER_API_KEY` set, also one open-mode run.
  - Record the status, seconds, tokens and cost (with its label) of each.
- [ ] **Step 2: Write the README:**
  - what the demo shows;
  - how to run it (`docker compose up -d`, `cargo run -p arenad`, `npm run dev`);
  - the two modes and what each records;
  - the fork rules as the user sees them;
  - the measured snapshot time from Task 4;
  - the live results from Step 1 exactly as measured;
  - what isn't built yet (the spec's non-goals);
  - τ²-bench attribution.
- [ ] **Step 3:** Run `cargo test` and `cd ui && npx vitest run && npm run build`. Expected: all PASS. Final checkpoint.
