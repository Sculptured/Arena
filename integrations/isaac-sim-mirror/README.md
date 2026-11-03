# isaac-sim-mirror

Follows a factory plant that is already running, and puts its state and events in the world. It does not make a
plant, has no tools, and only ever sends GET requests. Restore: none. Exact is `none` and approximate is false, so
arenad refuses to rerun a world that contains it, in either mode (`core/crates/arenad/src/restore.rs`).
`uses_clock`: true (each `advance` reads the plant's new events). Verified offline only: a pytest test
(`test_mirror_mode_has_no_tools_and_relays_events` in `integrations/isaac-sim/tests/test_part.py`) runs it against
a stub HTTP server. It has never followed a real plant, and never an Isaac Sim one. That test is in the isaac-sim
suite, which is skipped without the factory example; the example is not public yet (see the isaac-sim README).

It is the `isaac-sim` program run with `--mode mirror`, so its code and full notes are in
`integrations/isaac-sim/` (see "Mirror mode" in its README).

- Config: `plant_url` (default `http://127.0.0.1:8090`).
- `start` reads `GET /state` and follows from its `log_seq`. Events before the start are not this world's.
- `advance` returns `GET /events?since=<last seq>` as world events.
- `inspect` shows pieces of `GET /state`. `save` stores the current state for display only (`exact: false`);
  nothing can start from it.
- No `[[example]]` in the manifest, so `arena-part-check` can't run on it.
