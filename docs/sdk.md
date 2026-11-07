# Use Arena from code

Arena's core is **arenad** (the service), the **`arena-part/1` protocol** and its **kits**. It is headless and
fully usable without a UI: everything the web UI does goes through arenad's public HTTP API
([reference.md](reference.md#http-api)), and so can your code. Three ways in:

- **Python:** the `arena` package (`arena-sdk`, in [`sdk/python`](../sdk/python)): connect to or start a core,
  build environments, run teams, observe, control, export, and a step-loop environment for RL research.
- **Any language:** the HTTP API and the event stream (Server-Sent Events); the agents' endpoints are MCP and
  OpenAI chat completions, so any MCP client and any OpenAI-compatible client work.
- **Rust:** the same HTTP API ([below](#rust)), or arenad as a library.

The rule for agents everywhere: they are real models. Arena never supplies a scripted agent; your code, Claude
Code, Codex, or an API model does the work, and Arena records, restores and scores it.

## Install

```bash
cd sdk/python && uv sync          # or, from another project:
uv add "arena-sdk @ git+https://github.com/Sculptured/Arena#subdirectory=sdk/python"
```

Python 3.10 or newer; one dependency, `httpx`. You also need a core: a running arenad, or a build of this
repository (`cargo build --release` at the root and in `core/`) plus Postgres (`docker compose -f
deploy/compose.yml up -d`) for `Arena.local`.

## Connect or start a core

```python
from arena import Arena

arena = Arena("http://127.0.0.1:4510")                # a running arenad

with Arena.local(                                     # or start one for the length of the block
    benchmark="retail",                               # ARENA_BENCHMARK
    bin_dir="core/target/release",                    # where arenad and the package programs are
    packages=["integrations", "benchmarks"],          # ARENA_PACKAGES (default: arenad's own)
    pg_url="postgres://arena:arena@127.0.0.1:5544/postgres",
    saves="runs/saves",                               # keep saves to fork later (default: a temp folder)
    env={"ARENA_LOCAL_MODELS": "ollama=http://127.0.0.1:11434/v1"},
) as arena:
    print(arena.health()["benchmark"])
```

An arenad runs one benchmark. To use several, start one `Arena.local` per benchmark (each on its own port; they
can share one Postgres server). `arena.health()`, `arena.providers()` and `arena.world()` say what the core can
do: which CLIs it found, which model providers have keys, what each part of the world can restore.

Reads are retried when arenad can't be reached (it may be restarting); every error from arenad is an
`ApiError` subclass carrying its status and message: `BadRequest`, `NotFound`, `Conflict` (a running branch, a
part that can't be restored), `DiskFull` (the saves' limit).

## Build an environment

An environment is a benchmark's task, a seed, and optionally a scenario: disruptions the benchmark offers in
`scenario_options`, each with params.

```python
bench = arena.benchmark()                # name, version, tasks, scenario_options, view
task = arena.task(bench.tasks[0].id)     # ticket and instructions
print(bench.disruptions)                 # [{id, label, description, part, params: [...]}]

from arena import Disruption
branch = arena.run(task, team, seed=7,
                   scenario=[Disruption("courier-capacity", {"after": 2, "capacity": 0})])
```

The world itself is whatever the benchmark names: its parts and their seed data. To assemble your own world from
part packages, write a benchmark package that lists the parts (`describe.world`) and brings the tasks, tools and
gold actions or named checks ([packages.md](packages.md#write-a-benchmark)), put it in a folder, and pass that
folder in `packages=[...]`. The core has no route to compose a world on the fly.

## Run teams

**Arena's own runners.** arenad starts each agent: Claude Code or Codex headless with only the world's tools
(CLI mode), or its own loop with a provider's model (API mode). `chain` runs them one after another, `concurrent`
all at once.

```python
from arena import Agent, Team

team = Team([
    Agent.claude_code("triage", model="haiku", prompt="Find the customer and the orders.", read_only=True),
    Agent.api("resolver", provider="openrouter", model="deepseek/deepseek-chat-v3.1"),
])
branch = arena.run(task, team, runner="chain")
team = arena.default_team(model="haiku")    # or the benchmark's suggested team
```

**Your own agents (external runs).** Your code drives the agents in whatever shape your design has; arenad
records everything they do through their endpoints and scores the world when you finish.

```python
branch = arena.external_run(task, agents=["planner", Agent.external("executor", provider="openai", model="gpt-4.1-mini")])
ex = branch.agents["executor"]
ex.mcp_url          # MCP (streamable HTTP): the world's tools + handoff, message, inbox
ex.gateway_url      # OpenAI-compatible base URL: calls go to the agent's provider, recorded
ex.tools            # [Tool(name, description, input_schema)]; .openai_schema(), .anthropic_schema()
ex.call("get_order_details", order_id="#W5270061")    # one tool call, recorded like any agent's
ex.mcp_config()     # {"mcpServers": {"arena": {"type": "http", "url": ...}}} for Claude Code and MCP hosts
ex.report_usage(input_tokens=1200, output_tokens=80, model="...")   # model calls made outside the gateway
ex.end(ok=True, prompt_tokens=..., completion_tokens=..., cost_usd=...)
branch.finish()     # freeze, score, stop the world: {"status": "passed"|"failed", "error": ...}
```

Plug these into any framework: an OpenAI client with `base_url=ex.gateway_url` (the gateway holds the key, which
is set in arenad's environment), any MCP client that speaks streamable HTTP pointed at `ex.mcp_url` (arenad
answers each request with JSON), or Claude Code: `arena.claude_code.run(ex, prompt, model="haiku")` starts the CLI on that endpoint only (no built-in
tools), records each model call it reports and the agent's end. Agents can join at any time
(`branch.register(...)`). See `examples/bring_your_own_agent.py`.

## Observe

```python
for event in branch.stream():        # live: the events so far, then each new one, until the branch ends
    print(event.summary())           # event.kind, .seq, .agent, .payload; .tool, .error, .writes
branch.final                         # {"status", "error"}

async for event in branch.astream(): ...

rec = branch.record()                # status, team, events, metrics, fork info, run_start (seed, tools, systems)
rec.check                            # CheckResult: passed, checks [NamedCheck(name, passed, detail, safety)],
                                     #   score, safety_violations, differing_rows
rec.metrics                          # measured_s, estimated_s (without Arena's overhead), tokens, cost, per agent
branch.views()                       # the world's view after each step (the digital twin), if the benchmark draws one
branch.rows(seq=12, keys=["orders/#W5270061"])     # state of keys just after step 12
```

## Control

```python
branch.finish()                                  # external runs: score and stop
if arena.supports_pause():                       # newer cores: POST /api/branches/{id}/pause, /resume, /stop
    branch.pause(); branch.resume(); branch.stop()
else:
    ...                                          # branch.pause() raises NotSupported

from arena import Change
fork = branch.fork(17, changes={"resolver": Change(prompt="...", model="sonnet")}, restore="exact")
fork.fork_info                                   # start_seq, moved_from, reason (a CLI agent restarts from its start)
branch.restore_plan(17)                          # how each part will be restored, or the refusal
fork = ext_branch.fork(9, team=[Agent.external("solo")])     # external runs: another team, live, with history
arena.compare(branch, fork)                      # first divergent step, state that differs at the end

suite = arena.suite(team, tasks=["27", "72"], repeats=5)     # arenad runs them; or runner="external" + claim()
suite.wait(); suite.summary                      # pass rate, score, safety violations, spread
arena.compare_suites(before, after)["verdict"]   # improved, regressed, same, incomparable
```

## Export

```python
branch.export("data/run-72")        # events.jsonl, branches.jsonl, SCHEMA.md
suite.export("data/suite-a")        # every run of the suite, plus suite.json
```

The format (`arena-dataset/1`) is documented in [`sdk/python/src/arena/SCHEMA.md`](../sdk/python/src/arena/SCHEMA.md),
copied into every export. `pd.read_json("events.jsonl", lines=True)` loads it.

## A step-loop environment for RL

`arena.env.ArenaParallelEnv` wraps an external run in PettingZoo's parallel API (without depending on it). Your
policy chooses each agent's tool call; Arena runs it in the world, records it, and scores the episode.

```python
from arena.env import ArenaParallelEnv, END

env = ArenaParallelEnv(arena, task="72", agents=["support"], max_steps=30, reward="score",
                       shaping=lambda env, agent, action, obs: -0.01)        # optional per-step reward
obs, infos = env.reset(seed=3)        # obs[a]: prompt, ticket, instructions, tools
while env.agents:
    actions = {a: policy(obs[a]) for a in env.agents}   # {"name": tool, "arguments": {...}}, None, or END
    obs, rewards, terminations, truncations, infos = env.step(actions)
    # obs[a]: tool_result {text, is_error, value}, messages (notes and messages to a), world_events
env2, obs2, infos2 = env.fork(at_seq=5)   # "reset to step N": a live fork, world restored, history in obs2
env.close()
```

The terminal reward is the pass (1.0/0.0) or the share of goal checks passed, given to every agent at the end;
`infos[a]["check"]` has the checks and safety violations. Actions within a step run in `possible_agents` order, so
a step is reproducible given the world. `examples/step_env.py` uses a Claude model as the policy.

## Reproducibility

- **The same world.** Every run of a task starts from the same base world, made once with a fixed seed
  (`base_seed`, recorded) and saved; the task's gold world is made once from it. A run's own `seed` reaches the
  parts that draw from it (simulators, clocked parts): set it with `seed=` to repeat it; it is recorded in
  `RunStart` either way.
- **What varies is the models.** LLMs are not deterministic, and CLI agents add their own variation. Measure
  rates, not single runs: a suite with `repeats`, and `compare_suites` for a change. Record the model ids you used
  (they are in each branch's team and model calls).
- **Exact and approximate restores.** A fork puts each part back as it was before the fork step: exactly (a save,
  or a deterministic replay checked against the record) or approximately (a save that may miss something). A
  fork that can't restore a part in the mode asked is refused, naming it; an approximate fork is labelled
  (`restore_mode`, `restored`). See [concepts.md](concepts.md#restore-modes).
- **Pinned parts.** A replay is refused when a part's fingerprint (versions, source hashes) changed since the
  run; each part's fingerprint is in `RunStart.fingerprints`.
- **Time.** Arena times its own overhead on every event; `estimated_s` is the run's time without it.

## Add your own world part

A part is an integration: a program in any language that speaks `arena-part/1` over stdin and stdout. Start from
the counter in [integrations/README.md](../integrations/README.md) with the Rust kit (`kits/rust`) or the Python
kit (`kits/python`, `kits/python/examples/counter.py`), check it with `arena-part-check`, and name it in a
benchmark's world ([packages.md](packages.md)). Then pass the folders to `Arena.local(packages=[...])`.

## Rust

arenad is a library crate as well as a binary (`core/crates/arenad/src/lib.rs`): its modules (`app`, `api`,
`runner`, `fork`, `store`, ...) can be embedded, as its own tests do (`app::router(state)` gives the whole
service as an axum `Router`). That API is internal and changes without notice; for a stable interface use the
HTTP API, for example with `reqwest`:

```rust
let c = reqwest::Client::new();
let base = "http://127.0.0.1:4510";
let run: serde_json::Value = c.post(format!("{base}/api/runs"))
    .json(&serde_json::json!({"task_id": "72", "team": {"id": "t", "name": "t", "agents": []},
                              "runner": "external", "seed": 7}))
    .send().await?.error_for_status()?.json().await?;
let branch = run["branch_id"].as_str().unwrap();
let agent: serde_json::Value = c.post(format!("{base}/api/branches/{branch}/agents"))
    .json(&serde_json::json!({"id": "a"})).send().await?.json().await?;
// agent["mcp_url"], agent["gateway_url"]; then POST {base}/api/branches/{branch}/finish
```

A typed Rust client crate is not written yet. Write integrations and benchmarks with the Rust kit (`kits/rust`).

## Core and UI are separate

The core (arenad, the protocol, the kits, the integrations and benchmarks) depends on no UI. The web UI (`ui/`)
is one client of the HTTP API, as Sculptured's Arena control and this SDK are. arenad can serve a built UI
(`ARENA_UI_DIST`) as a convenience; it is not required. A UI served from another origin can call the API when
arenad lists that origin in `ARENA_CORS_ORIGIN` ([reference.md](reference.md#configuration)); the API still
answers only a loopback `Host`.

## Cite Arena

If Arena helps your research, cite it (see [CITATION.cff](../CITATION.cff)):

```bibtex
@software{arena,
  title  = {Arena: a rewindable staging environment for multi-agent systems},
  author = {{Sculptured}},
  year   = {2026},
  url    = {https://github.com/Sculptured/Arena},
  license = {Apache-2.0}
}
```
