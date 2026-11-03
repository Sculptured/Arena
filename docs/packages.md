# Packages: integrations and benchmarks

Everything system-specific in Arena is a package: a folder with an `arena-package.toml` and a program that
speaks [`arena-part/1`](protocol.md). arenad finds packages under the `ARENA_PACKAGES` roots (default
`integrations/` and `benchmarks/`) and starts their programs itself. The core never depends on a package, and a
package never depends on the core: it uses only the kits.

## Write an integration

A folder with an `arena-package.toml` (`kind = "integration"`, `name`, `version`, `protocol`, `run`, `needs`,
`config_schema`, and `[[example]]` tool calls) and a program that speaks the protocol. The Rust kit
(`kits/rust`, crate `arena-part`) and the Python kit (`kits/python`, package `arena_part`) read and write the
lines and check the ids; you fill in `describe`, `start`, `call_tool`, `advance`, `save`, `inspect`, `diff`,
`drop_save`, `stop` and `usage` (`gc` and `connect` are optional). `integrations/README.md` walks through the
counter example (`kits/rust/examples/counter.rs`, `kits/python/examples/counter.py`). Then check it, from the
repository root:

```bash
cargo build                                                  # your integration (debug build)
(cd core && cargo build -p parts --bin arena-part-check)     # the checker
ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/<name> [--config file.json]
```

`arena-part-check` runs the program and checks describe, start, the examples, save and restore, diff, replay
and `drop_save`. It prints a JSON report and exits 0 if all passed, 1 if not, 2 if it could not run.

Install a package by putting its folder under one of the `ARENA_PACKAGES` roots. A `command` run is for
programs you trust; it runs in the package folder with only `PATH`, `HOME`, `ARENA_PG_URL`, `UV_CACHE_DIR`,
`XDG_CACHE_HOME`, `RUST_LOG` and `ARENA_SAVES` in its environment. An `image` run is boxed in a container: no
network unless the manifest asks for host networking (`needs.network = "host"`), no environment from arenad,
and only its own saves folder, mounted at `/saves`.

## Write a benchmark

A benchmark is a package with `kind = "benchmark"` in its `arena-package.toml` and a program that speaks the same
transport (`docs/protocol.md`, "Benchmark calls"). It answers:

- `describe`: its name and version, the world (`[{part, integration, config}]`), the tasks (`[{id, reason,
  ticket}]`), the instructions every agent gets, and its tools (MCP shape plus `effect: "read"|"write"`).
  Optionally `instructions_title`, the heading over the instructions in each agent's input (default
  `Instructions`), and `team`, the team it suggests (`{id, name, agents: [{id, name, prompt}]}`).
  The CLIs know the world's MCP server by the benchmark's name.
- `seed`: load the starting data into fresh instances. arenad saves the result once as the base world that
  every run starts from.
- `call_tool`: run one of its tools and say which part it touched and which keys it read and wrote.
- `gold_actions`: the tool calls that solve a task. arenad applies them to the base world once per task to make
  the gold world that branches are scored against.

A benchmark's tools reach parts only through the parts' `connect` info (for `postgres`, `{url}`), so only parts
with exact saves may be connected. In Rust, implement `arena_part::Benchmark` and end `main` with
`serve_benchmark(..)`; the Python kit has no benchmark support yet.

`benchmarks/retail` is the included example: a Rust program (`src/main.rs`, `src/lib.rs`) whose world is one
`postgres` part named `retail`; `src/load.rs` seeds it from `data/retail/db.json`, `src/tools/` holds the tools,
and `src/tasks.rs` reads the tasks and gold actions from `data/retail/tasks.json` (the tasks offered are the
ids in `data/retail/demo_tasks.json`); `src/team.rs` holds the team it suggests and the `Store policy` heading
over its policy. Its tests are in `benchmarks/retail/tests`. arenad runs one benchmark at a time, chosen with
`ARENA_BENCHMARK`.
