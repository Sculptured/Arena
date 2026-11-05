# Factory showcase: three agent designs on one plant failure, measured by Arena

A press bearing fails at Ridgeline Components, a fictional stamping plant (the `factory` benchmark). The
incident crosses an ERP (ERPNext), its MES records (work orders and job cards on the press-line
workstations), a CRM (Twenty), the simulated plant, the suppliers, carriers and customers, and an approvals
system. Three designs handle it, each on Claude Code headless, with the same model, world, seed and tools:

- **A, hierarchical**: an Incident Commander delegates to Maintenance, Procurement, Production planning and
  Customer communications by Arena's `message` tool, and decides every change.
- **B, event-driven**: the same four roles start at once and act on topics (`#press.down`,
  `#diagnosis.ready`, `#approval.requested`, `#approval.decided`, `#po.approved`, `#eta.changed`,
  `#schedule.ready`) sent with Arena's `message` tool to `*`; the design's code delivers a topic only to the
  roles subscribed to it. Production planning decides the changes.
- **C, single agent**: one agent with every role's instructions and every tool.

Everything is fictional and simulated: the company, its customers and orders, the plant (the stand-in
plant, without physics) and the outside companies. ERPNext and Twenty are real open-source systems.

RESULTS_PLACEHOLDER

## What is where

| Path | What |
|---|---|
| `designs/designs.py` | The three designs: roles, prompts, topics, end conditions |
| `designs/claude_agent.py` | One agent as a long-lived `claude -p` process (stream-json in and out), with arena's own driver's isolation flags; reports each model call's usage and each compaction to Arena |
| `designs/usage.py` | Counts an agent's model calls from Claude Code's stream once each, with the real output tokens and the calls the stream never shows (a compaction's), so Arena's totals match what Claude Code billed |
| `designs/engine.py` | Routes Arena's recorded messages to the agents, wakes waiting agents, ends the run |
| `designs/run.py` | Runs a design (or a fork of a recorded run) through Arena's external-run API |
| `designs/test_engine.py` | Offline tests of the engine (no model, no Arena) |
| `export_metrics.py` | `metrics.json` for the video (`arena-video`'s `video/README.md`), from Arena's API only |

The designs live here, outside Arena's core: Arena only records. What Arena itself gained for this showcase is in
[docs/reference.md](../../docs/reference.md): `POST /api/branches/{id}/agents/{agent}/usage` (a CLI agent's per-call
tokens and context, and its compactions, recorded as `model_call` events), each agent's `model_calls`,
`peak_prompt_tokens`, `compactions` and `overhead_s` in a branch's metrics, and the ticket and instructions in
`GET /api/tasks`.

## Recreate the setup

On Linux with Docker, Rust, Python 3.11+, `uv` and Claude Code signed in (`claude` on `PATH`). The factory
example (`Sculptured-Examples/factory`, next to the arena checkout) and a current Sculptured program are needed and
are not public yet (see `benchmarks/factory/README.md`). Images: `frappe/erpnext:v16.53.0`, `mariadb:11.8`,
`twentycrm/twenty:v2.51.0`, `postgres:17-alpine`, `redis:6.2-alpine` (none is pulled for you).

```bash
# 1. Where the build and the saves go (this machine used /home/alexsky2/sculptured-work-local).
W=/path/to/work
export CARGO_TARGET_DIR=$W/target-arena ARENA_BIN_DIR=$W/target-arena/debug ARENA_SAVES=$W/saves
export ARENA_SCULPTURED_BINARY=/path/to/sculptured PATH=$(dirname $ARENA_SCULPTURED_BINARY):$PATH
export ARENA_BENCHMARK=factory

# 2. Arena's Postgres, the packages, and the base world (about 30 minutes the first time: it seeds
#    1,083 orders into ERPNext and Twenty).
docker compose -f deploy/compose.yml up -d
cargo build -p arena-factory -p arena-erpnext -p arena-twenty-crm -p arena-sculptured
(cd core && cargo run -p arenad -- base)

# 3. arenad, with the plant clock at real time (1, the default) and the stand-in plant.
(cd core && ARENA_UI_DIST=$PWD/../ui/dist cargo run -p arenad)
```

## Run the designs

From `examples/factory-showcase/designs`, with arenad up:

```bash
python run.py --design A --out $W/runs/A-1      # likewise B and C
```

Each run starts an Arena run with `runner: "external"`, starts one Claude Code process per agent on its
Arena MCP endpoint, and finishes the run when the design's end condition holds (or after `--max-minutes`, 120 by
default). `$W/runs/<name>/` keeps each agent's raw stream-json, its stderr, the engine's log and `meta.json`
(the run and branch ids, how it ended).

The model is `claude-sonnet-4-5` for every agent of every design (`--model`); its context window is 200,000
tokens (`modelUsage.contextWindow` in Claude Code's answer). The current Claude Code models (`sonnet` is
`claude-sonnet-5-5`, also `opus` and `haiku`) report a window of 1,000,000 tokens, which this task would not
strain, so the showcase uses the 200K model, as the brief asked.

## Fork and compare in Arena

```bash
python run.py --design C --out $W/runs/C-fork --fork-of <branch> --at-seq <seq> --change incident=fix.txt
curl -s "http://127.0.0.1:4510/api/compare?a=<branch>&b=<branch>"
```

## Metrics for the video

```bash
python export_metrics.py --design A=<branch>,<branch> --design B=... --design C=... --fork <branch> \
    --out $W/runs/metrics.json
```

## Tests

```bash
(cd designs && python -m unittest test_engine test_usage)      # the engine and the usage meter, offline
cargo test -p arena-factory -p arena-erpnext -p arena-twenty-crm # the benchmark and the integrations
(cd core && cargo test -p arenad --test external)               # the usage endpoint (needs Arena's Postgres)
```
