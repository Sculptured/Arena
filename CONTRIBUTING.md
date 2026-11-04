# Contributing to Arena

Thanks for helping. Bug reports, new integrations and benchmarks, docs fixes and code are all welcome.

## Before you start

- **Bugs and ideas:** open an issue with one of the templates. For anything large, such as a protocol change or a new kind of restore, open an issue first so we can agree on the approach before you write code.
- **A new integration or benchmark:** open a "New integration" issue to say what you are building; it can live in this repository or in your own.
- **Security problems:** do not open an issue; see [SECURITY.md](SECURITY.md).
- Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md).

## Set up

You need [Rust](https://rustup.rs/) (stable), [Node.js](https://nodejs.org/) 24, Docker, Git and, for the Python kit and integrations, [uv](https://docs.astral.sh/uv/). Claude Code or Codex is only needed to run real CLI agents, and a model provider (a key such as `OPENROUTER_API_KEY`, or a local server in `ARENA_LOCAL_MODELS`) only for API agents; the tests call neither.

```bash
git clone https://github.com/Sculptured/Arena.git
cd Arena
docker compose -f deploy/compose.yml up -d     # Postgres on 127.0.0.1:5544, used by arenad and the tests
cargo build --release                          # integrations and benchmarks, into core/target/release
(cd core && cargo run -p arenad)               # API on http://127.0.0.1:4510
(cd ui && npm ci && npm run dev)               # UI on http://127.0.0.1:4520
```

## Repository layout

| Path | What it holds |
|---|---|
| `core/` | arenad (`crates/arenad`) and the package host and checker (`crates/parts`, `arena-part-check`). No code for any particular system. |
| `kits/rust`, `kits/python` | The kits integrations and benchmarks are written with: crate `arena-part`, package `arena_part`. |
| `integrations/<name>/` | One package per kind of system (Postgres, a folder, Docker Compose, MariaDB, ERPNext, ...). |
| `benchmarks/<name>/` | Benchmarks: tasks, tools, seed data and gold actions. `benchmarks/retail` is the included example. |
| `ui/` | The web UI (React, Vite). |
| `docs/` | The protocol, concepts, reference and measurements. |
| `deploy/compose.yml` | The Postgres arenad and the tests use. |

The core never depends on an integration or a benchmark, and they never depend on a `core/` crate: they use only the kits. `core/crates/parts/tests/isolation.rs` checks this.

## Make a change

1. Branch from `main`.
2. Write a test that fails without your change, then make it pass.
3. Run the checks (with the Postgres above running, and arenad stopped: its startup cleanup drops databases the tests make):
   ```bash
   cargo fmt --all --check && (cd core && cargo fmt --all --check)
   cargo clippy --workspace --all-targets && (cd core && cargo clippy --workspace --all-targets)
   cargo test && (cd core && cargo test)
   (cd kits/python && uv run --with pytest pytest -q)
   (cd ui && npm test && npm run build)
   ```
   The tests of `integrations/ridgeline-hub` and `integrations/isaac-sim` read the Ridgeline factory example from `Sculptured-Examples/factory` next to your checkout. **That example is not public yet**, so outside contributors can't run those two suites: without it every test in them is skipped. CI runs everything above and only imports those two packages. Likewise the Sculptured program that `integrations/sculptured` runs is not published yet; its only test is `live_` and is skipped without it. `benchmarks/factory` needs both: its unit tests run anywhere, and its `live_` test skips without them.
4. If you changed a test that makes databases, run `core/scripts/leakcheck.sh` from `core/`: it fails if a test leaves databases behind.
5. If you changed the UI, look at it in the running app and say in the pull request what you checked.
6. Open a pull request and fill in the template.

Tests that need Docker services, real servers, GPUs or real models are `#[ignore]`d with the prefix `live_` (Rust) or marked `live` (Python). Run them when you change the code they cover, and say so in the pull request: `cargo test -- --ignored live_` in this folder starts containers; `(cd core && cargo test -- --ignored live_)` calls the real Claude Code and Codex CLIs and costs money.

## The protocol

arenad talks to every integration and benchmark over `arena-part/1`: JSON-RPC 2.0, one message per line, on the program's stdin and stdout. [docs/protocol.md](docs/protocol.md) is the specification. A change to the protocol updates `docs/protocol.md`, the kits and `arena-part-check` together, in one pull request.

## Write an integration

An integration is a folder with an `arena-package.toml` and a program, in any language, that speaks the protocol. [integrations/README.md](integrations/README.md) walks through the counter example step by step. Check it with the conformance checker before you open a pull request:

```bash
cargo build                                                  # your integration (debug build)
(cd core && cargo build -p parts --bin arena-part-check)     # the checker
ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/<name> [--config file.json]
```

Give the integration a README that says what it puts in the world, its config, how it saves and restores, and what was verified and on what. Never claim a restore mode the checker has not passed.

## Write a benchmark

A benchmark is a package with `kind = "benchmark"` that names its world and answers `describe`, `seed`, `call_tool` and `gold_actions` ([docs/protocol.md](docs/protocol.md), "Benchmark calls"). In Rust, implement `arena_part::Benchmark` and end `main` with `serve_benchmark(..)`; `benchmarks/retail` is the example to copy, and `benchmarks/factory` shows links, gates and named checks. Credit and license any third-party data you include, as `data/retail/LICENSE` does.

## Code rules

These are the rules reviewers hold changes to; [AGENTS.md](AGENTS.md) has the same list for coding agents.

- **Modules have one responsibility.** Put implementation in leaf modules and keep public import paths stable with re-exports when you split a module.
- **Functions stay focused and short.** `clippy.toml` sets `too_many_lines` to 30. It is advisory, not a reason to split coherent code: extract helpers around meaningful responsibilities. Never add `#[allow(clippy::too_many_lines)]` or raise the threshold to make a change pass.
- **DRY, without speculative abstractions.** Share repeated logic in one place.
- **Idiomatic Rust.** Prefer borrowing to deep clones, named structs to ambiguous tuples, and pattern matching to check-then-unwrap. Use `expect` with the invariant spelled out when a panic is required.
- **Python follows the same rules:** short functions, one responsibility per module, no copy-pasted logic.
- **Tests never call a real model** unless they are `live_`.
- **Tests clean up after themselves:** every database, container and folder they create is dropped, even when an assertion fails.
- **One target directory.** Both Cargo workspaces build into `core/target` (`.cargo/config.toml`), where arenad finds the programs; never add another.
- **Say only what was verified.** Docs and READMEs keep what was run, and on what, separate from what is designed or assumed.

## Commit messages

Start the subject with the area it changes and say what the change does: `arenad: register live branch before releasing the cleanup guard`, `postgres: ...`, `ui: ...`, `docs: ...`. Explain why in the body.

## License

By contributing you agree that your contributions are licensed under the [Apache License 2.0](LICENSE).
