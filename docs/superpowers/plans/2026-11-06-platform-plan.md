# Arena + Sculptured platform plan (2026-11-06)

User, 2026-11-06: "anything should be able to hook into our platform, simulate the world, and have the agents
operating in it, then someone can see how the agents performed, reverse time, make changes, rerun a simulation
with the same seeds but different agent architectures, different scenarios they construct over time,
architectures from communities, context management tooling" and "build those things. Go through all of the
code and make sure it is modular, DRY, clean, little functions ... build each of these features one at a time,
make sure the UI is super clean and clear, use very good UI tools, write down a plan of action for each system
first and then go one by one."

**Direction:** Arena is the experimentation and qualification environment for agent systems in any industry.
Sculptured is one way to design and run those systems, and Arena does not depend on it. The Boston flagship is
the pharma production world; insurance claims is the second reference world.

**How each system is built:**
1. A short design note, added to this file's section for the system.
2. Implementation by Codex, in a worktree off the integration branch.
3. A controller check of the diff and tests, then a merge into the integration branch.

Nothing is pushed or merged to `main` without the user's word. No agent runs with real models (credits):
tests use scripted agents and fake model servers.

**Integration branches:** arena `platform` (from `feature-tour`, which already has factory-benchmark,
pharma-showcase, suites and pharma-world) and sculptured `platform` (from `control-plane`).

## Shared standards (every system)

- **Rust:** clippy `-D warnings` with `too_many_lines` at 30, one responsibility per module, no duplicated
  logic, named structs instead of tuples, `expect` with the invariant stated.
- **TypeScript:** an ESLint `max-lines-per-function` rule of 40, with a complexity limit added to both UIs.
  Components only render; logic lives in tested pure modules.
- **Python:** ruff with `max-statements` / `PLR0915` and `C901`.
- **Duplication:** `jscpd` reports for both repos, with a budget enforced in CI.
- **UI toolkit, both UIs** (one look, the industrial-HMI style already chosen):
  - Radix UI primitives for dialogs, menus, tabs, tooltips and popovers (accessible and unstyled);
  - `@xyflow/react` (React Flow) for graphs: the process view, team boards and scenario timelines;
  - uPlot for dense time series (trends);
  - TanStack Table for grids (suites, leaderboards, events).

  Styling stays on each repo's design tokens (CSS variables); no CSS framework migration.
- **Docs** say only what was verified.

## System 0: consolidate branches

Make arena `platform` and sculptured `platform`. Run every test suite once on each, then cut the current
state of the factory-showcase video work by the other session into its own branch, so it isn't lost or
clobbered. **Done when** both platform branches are green.

## System 1: code health pass (both repos)

**Plan of action:**
- Turn on the TypeScript and Python length and complexity rules above, plus `jscpd`, and produce a report of
  every violation per repo.
- Fix in batches by module: split long functions along real responsibilities, extract shared helpers, and
  remove dead code.
- Rust: clippy is already enforced, so look for duplicated logic across crates. For example, the kit and
  core helpers, and `integrations/*` that each re-implement save directories or compose handling.
- Not done: churn for its own sake; renames that touch public APIs without need.

**Done when:** the lint rules are on in CI with zero violations, the duplication budget is set from the new
baseline, and all tests are green.

## System 2: the same conditions for every architecture (causal determinism)

**Problem:** a shared seed isn't enough. Two architectures that act differently consume random numbers in
a different order and trigger events at different times, so they face different worlds.

**Plan of action:**
- **Random streams.** The kits get named, seeded random streams per world process, for example
  `rng("weather")` and `rng("supplier:hartwell")`, derived from the run seed and the stream name, so one
  process's draws never shift another's. Parts use streams, never a global generator.
- **Outside events.** Events that don't depend on agents (failures, deliveries, price moves, arrivals) come
  from a *disturbance schedule*: generated from the scenario and seed before the run, recorded in `RunStart`,
  and applied on the world clock, whatever the agents do. Events that do depend on agents (a reply to an
  agent's request) draw from their own stream.
- **Run modes, labelled.** `replay` repeats recorded events and model answers (exact). `counterfactual`
  re-simulates with the same schedule and streams but new agent behaviour. The UI says which one a run is.
- **Nondeterminism made explicit.** Model answers and real external APIs are marked as nondeterministic
  sources in the record, unless captured and replayed.
- **Tests:** two scripted architectures making different call patterns see the same disturbance times and
  values; a replay is byte-identical; a part that uses a global generator is caught by `arena-part-check`.
- **UI:** in the run view, a "conditions" strip showing the disturbance schedule on the timeline, marked
  "same as run X" when two runs share it.

**Done when:** the factory and pharma worlds use streams and schedules, and two different suites provably
face identical conditions.

## System 3: scenarios people build over time

**Plan of action:**
- **Scenarios as versioned data, not code.** A scenario file (YAML) names the world (benchmark and version),
  initial conditions as overrides of the base (setup calls), the disturbance schedule as rules (what, when,
  how random), the checks it enables, and a seed policy.
  - Stored in Arena with an id and a version (content hash); edits make a new version.
  - The benchmark validates a scenario against the world's schema.
- **API:** create, list, version history, diff two versions, and run (a single run or in a suite).
- **UI:** a scenario builder:
  - pick a world;
  - edit initial conditions with forms generated from the world's schema;
  - place disturbances on a timeline (React Flow);
  - preview the schedule for a seed;
  - save as a new version and compare versions.
- **Tests:** a scenario round trip (create, version, diff, run with scripted agents); an invalid scenario is
  refused with a clear message.

**Done when:** the pharma variants exist as scenario files, and a new variant can be made in the UI and run
in a suite without code.

## System 4: any agent framework

**Plan of action:**
- **A Python package `arena-agents`** with a thin client for Arena's external-run API: start, register agents,
  per-agent MCP endpoints, usage, end, finish, and suites claim.
- **Adapters** that run a team from a framework inside Arena with no change to the team's code:
  - OpenAI Agents SDK;
  - LangGraph;
  - CrewAI;
  - a plain MCP client.

  Each adapter maps the framework's agents to Arena agents, routes its tools through the agent's Arena MCP
  endpoint, and reports usage.
- **The same for TypeScript:** a client, and a Vercel AI SDK adapter.
- **Tests:** each adapter runs a two-agent team against a fake model server and the toy world, then
  shows up correctly in a suite.
- **Docs:** "Bring your own agents", one page per framework.

**Done when:** each adapter passes its test, and an example per framework is in `examples/`.

## System 5: context management as a swappable part

**Plan of action:**
- **A context policy** is a versioned component on an agent: history window, summarisation (when, by
  which model), tool-output trimming (keep the first N lines or a summary, with the full result on demand
  through a tool), shared reports (compact summaries injected instead of raw data), and a memory tool.
- **Applied** by Arena's built-in agent runner, and exposed to the adapters as middleware, so the same
  policy runs in any framework.
- **Recorded** per call: which policy and version, context size before and after, and what was trimmed.
- **Compared** in suites like any other change: suite A with policy v1 against suite B with policy v2.
- **UI:** an agent's context panel showing its size per call, stacked by history, tool output and
  reports, with the policy's actions marked.
- **Tests:** policies trim and summarise deterministically with fake models; usage is recorded.

**Done when:** at least three policies exist (full, windowed, report-first), and a suite compare shows their
effect on context and outcome.

## System 6: versioned building blocks, registries and leaderboards

**Plan of action:**
- **Stable ids and versions** for worlds (benchmark and version and fingerprint), architectures (a team spec
  with a content hash, Sculptured board export included), scenarios (System 3), evaluators (a check set
  version) and context policies (System 5).
  - Every run and suite records the exact versions it used.
- **Registry:** list, show and diff each kind. Import and export as files, so the Sculptured Community can
  publish and fetch them.
- **Leaderboards:** for a scenario set and an evaluator version, a table of architectures with pass rate,
  safety violations, cost, time and spread. Confidence intervals come from repeats (bootstrap), and the
  board shows when two entries are not distinguishable.
- **UI:** a registry browser; a leaderboard table (TanStack Table); an entry's page linking to its suites
  and runs.
- **Tests:** version identity (the same content gives the same id), the leaderboard math (intervals,
  ties), and import/export round trip.

**Done when:** the pharma flagship has a leaderboard of the deployments with intervals, built from
registry entries.

## System 7: insurance claims world (second reference world)

**Plan of action:**
- **A claims world:** policies, claims, adjusters' queue, a fraud-signal service, payments, customer
  messages, regulatory deadlines.
- **Systems, using open-source stand-ins where they exist:** ERPNext for payments and accounting, Twenty for
  the CRM, a simulated claims core labelled as standing in for systems like Guidewire, and a simulated fraud
  service.
- **Scenarios,** as System 3 files: a normal claim, a fraud ring with a data service down, a regulatory
  deadline, a catastrophe surge.
- **Checks,** with safety: no payment without the required approval, no payment on a flagged claim, and
  deadlines met.
- **Two reference architectures and a scripted gold path.**

**Done when:** its suites run with scripted agents, and it shows up in the registry and leaderboard.

## In parallel: the feature-tour video

Demo data from scripted agents in the pharma world (Codex task 5b), then captures of every feature, then
narration and the edit. It is updated as systems land; the first cut covers what exists today.

## Order

**0 → 1 → 2 → 3 → 4 → 5 → 6 → 7**, with the video in parallel. Each system ends with a short report to the
user: what was built, test results, and screenshots of the UI.
