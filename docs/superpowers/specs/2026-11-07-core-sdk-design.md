# Arena core, separate from the UI, constructable from code (2026-11-07)

Approved by the user in chat on 2026-11-07 ("Ok go do that"). Their words: "make the Arena UI separated from
the core which should be constructable with code so people can use its env as they please, that's why it's
open source so people in academia can also use that stuff".

## Hard rule: real models only
No scripted, demo, stub or fake agents anywhere, examples and tests included. Agent examples and agent tests
use real models (Claude Code / Codex CLIs, or an API provider when its key is set) and tolerate model
variance. A test may drive Arena's API directly to check a contract (build a world, start and finish a run,
fork, read records, call a tool endpoint to check a gate) as long as it doesn't pose as an agent.

## 1. Core and UI are separate products
- The core is arenad plus the parts protocol and kits: headless, fully usable with no UI, documented on its own
  (a "Use Arena from code" quickstart at the top of the README).
- The UI is a separate package (`ui/`) that points at any core by URL (configurable at build or run time,
  default same origin). arenad serving a UI build (`ARENA_UI_DIST`) stays as an optional convenience, not a
  requirement; docs say so. CORS for a UI served from elsewhere: allow a configured origin.
- Nothing in the core depends on the UI; the UI depends only on the public HTTP API.

## 2. A Python SDK (primary): `arena` (package `arena-sdk`), in `sdk/python/`
High-level, typed, small. Researchers should be able to, in a few lines:
- **Connect or start a core**: `Arena(url)` for a running arenad; `Arena.local(...)` starts one as a
  subprocess from a built binary with a given benchmark/packages folder, Postgres URL and saves dir, and stops
  it on exit (context manager).
- **Build an environment**: pick a benchmark and task, a seed, a scenario (disruptions from
  `scenario_options`, with params), or assemble a world from part packages where the core supports it.
- **Run teams**: start a run with a team (Arena's own runners: CLI or API agents with real models), or with
  `runner="external"` and register your own agents: each gets its MCP endpoint (the world's tools) and its
  OpenAI-compatible gateway URL, to plug into any agent framework.
- **Observe**: stream events as they happen (iterator / async iterator), read the full record of a branch,
  the twin's views, metrics, checks and safety results.
- **Control**: pause/resume (if the core has it; another branch is adding `POST /api/branches/{id}/pause`
  and `/resume`; feature-detect, don't assume), stop/finish, fork at a step with changes or another team,
  suites and suite comparisons.
- **Export**: a branch or suite as a dataset (JSONL of events with a schema doc) for analysis.
Typed models for the API objects (dataclasses or pydantic, pick one and keep it light), clear errors, sync
API first with an async variant only if cheap. Python ≥3.10, published layout with pyproject, uv-friendly.

## 3. A step-loop wrapper for RL research: `arena.env`
On top of the SDK, a multi-agent environment in the style of PettingZoo's parallel API: `reset(seed)` builds
and starts an external run and registers the agents; `step(actions)` takes `{agent: tool call}` (name +
arguments), performs them through each agent's endpoint, and returns observations (tool results, messages,
world events the agent may see), rewards (from the task's checks at the end, and an optional shaping hook),
terminations/truncations and infos; `close()` finishes the branch. Forking gives "reset to step N" for
counterfactual rollouts. The researcher's policy (an LLM or anything else they bring) chooses the actions;
Arena supplies no agent.

## 4. Rust
Document using `arenad` as a library (it already has `lib.rs`): a small typed client for the HTTP API in a
`kits/rust` or new `sdk/rust` crate only if it stays small; otherwise document the HTTP API for Rust users and
leave the client for later.

## 5. Examples and docs (for academia)
- `sdk/python/examples/`: quickstart (connect, run a team of Claude Code agents on the pharma or retail task,
  stream the timeline, print the checks); bring-your-own-agent (a minimal real-model agent loop over the MCP
  endpoint and the gateway, e.g. with the OpenAI client pointed at the gateway when a key is set, or Claude
  Code with the MCP URL); counterfactual fork; a suite; export to a dataset; the step-loop env with a real
  model as the policy.
- Docs: reproducibility (seeds, exact vs approximate restore, what varies: the models), how to add your own
  world part (pointing at the existing kits), citing Arena.

## Not now
Agent connections for deployed systems (Bedrock etc.: backlog), the UI rebuild (another branch), the
industrial demo.
