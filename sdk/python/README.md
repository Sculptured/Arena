# arena-sdk

Build, drive, observe and export [Shakedown](https://github.com/Sculptured/Arena) environments from Python.
Shakedown's core (arenad) is headless; this SDK talks to its HTTP API. No UI needed.

```bash
uv add "arena-sdk @ git+https://github.com/Sculptured/Arena#subdirectory=sdk/python"   # or: uv pip install -e sdk/python
```

```python
from arena import Arena, Agent

with Arena.local(benchmark="retail", bin_dir="core/target/release") as arena:   # or Arena("http://127.0.0.1:4510")
    branch = arena.run("72", team=[Agent.claude_code("support", model="haiku")], seed=7)
    for event in branch.stream():          # live, ends when the branch ends
        print(event.summary())
    print(branch.check())                  # passed?, named checks, safety violations
    branch.export("datasets/run-72")       # events.jsonl + branches.jsonl + SCHEMA.md
```

- **Connect or start a core:** `Arena(url)`, `Arena.local(benchmark=..., packages=..., pg_url=..., saves=...)`.
- **Build an environment:** `arena.benchmark()` (tasks, `scenario_options`), `arena.run(task, team, scenario=[...], seed=...)`.
- **Run teams:** Shakedown's runners (`chain`, `concurrent`; Claude Code, Codex or API agents with real models), or
  `arena.external_run(task, agents=["a", "b"])`: each agent gets an MCP endpoint and an OpenAI-compatible gateway
  for any agent framework (`arena.claude_code.run(agent, prompt)` runs Claude Code on one).
- **Observe:** `branch.stream()` / `astream()`, `record()`, `views()`, `rows()`, `metrics()`, `check()`.
- **Control:** `finish()`, `fork(at_seq, changes=..., team=...)`, `pause()`/`resume()`/`stop()` on cores that have
  them (`arena.supports_pause()`), suites (`arena.suite(...)`, `compare_suites`).
- **Export:** `branch.export(dir)`, `suite.export(dir)`: JSONL with a schema doc.
- **Step loop for RL:** `arena.env.ArenaParallelEnv`, PettingZoo-style `reset`/`step`/`close`; your policy picks
  the tool calls.

Full guide: [docs/sdk.md](../../docs/sdk.md). Examples: [examples/](examples/). Python 3.10 or newer; one
dependency (`httpx`).
