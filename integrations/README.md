# Integrations

An integration is a program that puts one kind of system in the arena's world: a database, a folder, an ERP, a
simulator. It works like a device driver. arenad (the core) knows nothing about the system. It speaks the
protocol `arena-part/1` to the program, and the program does the rest: start an instance, run tool calls, save
the state, restore it, show it, compare it. The protocol is in `docs/protocol.md`; read that for every method and
error code.

Each integration is its own folder here, with an `arena-package.toml`. It uses only the kits (`kits/rust`,
`kits/python`), never a `core/` crate, and the core never depends on it. `core/crates/parts/tests/isolation.rs`
checks this. Benchmarks (tasks, tools, a check) are separate packages under `benchmarks/`.

## The integrations

The table is in the main `README.md`, with what was verified. Each folder has a README of its own.

| Folder | What it puts in the world |
|---|---|
| `postgres` | a Postgres database per instance |
| `folder` | a folder of files per instance |
| `service` | any Docker Compose project per instance |
| `mariadb` | a MariaDB database per instance |
| `erpnext` | an ERPNext site per instance |
| `sculptured` | a Sculptured server (approvals) per instance |
| `ridgeline-hub` | the factory's outside companies, on the arena clock |
| `isaac-sim` | the factory plant on Isaac Sim, or a stand-in |
| `isaac-sim-mirror` | a read-only view of a running plant |

Examples to copy: `kits/rust/examples/counter.rs` and `kits/python/examples/counter.py`.

## Restore modes

Each integration says, in `describe`, how it restores:

- `exact: "save"`: it can save its state exactly and start from that save.
- `exact: "replay"`: it can't save exactly. arenad starts it again with the same seed and repeats every
  recorded input. The part must be deterministic for this to work.
- `exact: "none"`: no exact restore.
- `approximate: true`: it can also start from a save that may miss some state. A rerun that uses this is
  labelled "approximate restore".

A rerun asks for `exact` (the default) or `approximate`. `core/crates/arenad/src/restore.rs` holds the table.

## Write one: the counter, step by step

The counter holds one integer per instance. Its Rust code is `kits/rust/examples/counter.rs`.

1. Make a folder with an `arena-package.toml` (see `kits/rust/examples/counter/arena-package.toml`): `kind`, `name`,
   `version`, `protocol = 1`, `run`, `needs`, and one `[[example]]` per tool call the check may run.
2. Depend on the kit crate `arena-part`. Write `impl Part for Counter` and end `main` with
   `serve(Counter::new(..)).await`. The kit reads and writes the lines and checks ids.
3. `describe` returns the name, `restore: {exact: Save, approximate: false}`, `uses_clock`, a `fingerprint` and the
   tools. `add` has `effect: Write`, `get` has `effect: Read`.
4. `start` makes an instance. If `from` is set, read that save instead (a part that can't restore in that mode
   answers `-32001`).
5. `call_tool` runs `add` or `get` and lists the keys it read and wrote (`counter/value`).
6. `advance` moves the clock; the counter adds one per second, in whole seconds, so splitting the advances
   changes nothing.
7. `save` writes `state.json` into `dir` and returns `{bytes, exact: true}`. `inspect`, `diff`, `drop_save`, `stop`
   and `usage` are small. A part whose saves all sit in `ARENA_SAVES` reports `usage` 0.
8. Build it, then check it (below). Put the folder where arenad looks (`ARENA_PACKAGES`, default `integrations/`
   and `benchmarks/`) and start arenad.

Python is the same with `arena_part.Part` and `arena_part.serve(Counter())`. A manifest for it is
`kits/python/examples/arena-package.toml`.

## Check it

    (cd core && cargo build -p parts --bin arena-part-check)
    ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/<name> [--config file.json]

Run the second line from the repository root. Build the integration's own binary first (`cargo build` there).

It starts the program and checks, in order: `describe`, `start`, each `[[example]]` tool call, save and restore (in
each mode the part declares; exact must match, an approximate difference is reported but does not fail), `diff` of a save against itself and against the original, replay for a part that
declares it, and `drop_save` lowering `usage`. It prints a JSON report and exits 0 if all passed, 1 if not, 2 if
it could not run. A step it can't do is listed as skipped, with the reason. `ARENA_BIN_DIR` is where `${ARENA_BIN_DIR}`
in the manifest points (default: the release build). Without `ARENA_SAVES` it uses a temporary folder.

Some folders have a `check/` package: the same program with a config and examples made for the check (`postgres`,
`mariadb`, `erpnext`, `folder`).

## Rust helpers in the kit

`kits/rust` has helpers behind features: `folder` (copy a folder as a save, with hard links), `compose`
(Docker Compose projects and their volumes), `postgres`, and `mariadb` (binary-log saves, used by `mariadb` and
`erpnext`). `kits/python` has `folder`, `FixedStep` (a fixed-step clock), `saves` and `sources`.
