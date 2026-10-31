# Arena — instructions for agents working in this repo

The arena records multi-agent runs against a world made of integrations, saves the world at every
step, and reruns from any step. The design is in `docs/specs/2026-10-31-integrations-design.md`; the
protocol between arenad and an integration is `docs/protocol.md`.

## Layout and boundaries
- `core/` is arenad and the primitives. It contains no code for a particular system.
- `integrations/<name>/` and `benchmarks/<name>/` are isolated packages, like drivers. They use only
  the kits (`kits/rust`, `kits/python`) and never a `core/` crate; `core/` never depends on them.
  `core/crates/parts/tests/isolation.rs` checks this.

## Code quality
- Keep modules focused on one responsibility. Put implementation in leaf modules and keep public
  import paths stable with re-exports when splitting a module.
- Keep functions focused and short. `clippy.toml` sets the `too_many_lines` threshold to 30. It is
  advisory, not a reason to split coherent code: extract helpers around meaningful
  responsibilities. Never add `#[allow(clippy::too_many_lines)]` or raise the threshold to make
  a change pass.
- DRY: share repeated logic in one place, without speculative abstractions.
- Prefer borrowing to deep clones, named structs to ambiguous tuples, and pattern matching to
  check-then-unwrap. Use `expect` with the invariant spelled out when a panic is required.
- Python packages follow the same rules: short functions, one responsibility per module, no
  copy-pasted logic.
- Run `cargo fmt`, the relevant tests, and `cargo clippy --all-targets` for every changed crate.

## Tests
- Tests never call a real model. Tests that need Docker services or hardware not always present
  are `#[ignore]`d with the prefix `live_` (Rust) or marked `live` (Python).
- Tests drop every database, container and folder they create, even when an assertion fails.
- The disk is nearly full: share `core/target` (set in `.cargo/config.toml`), never add another
  target directory.
