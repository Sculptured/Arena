## What this changes

<!-- What a user or an integration author notices, and why. Link the issue: Fixes #123 -->

## How it was checked

- [ ] A test that fails without this change
- [ ] `cargo fmt --check`, `cargo clippy --all-targets` and `cargo test` pass in this folder and in `core/`
- [ ] Python (`uv run --with pytest pytest -q`) and UI (`npm test && npm run build`) checks pass, or not affected
- [ ] `arena-part-check` passes for a new or changed integration, or not applicable
- [ ] `live_` tests run for the code they cover (say which), or not applicable
- [ ] No real model calls in non-`live_` tests, and every database, container and folder a test makes is dropped
- [ ] Docs and READMEs updated, and they say only what was verified

<!-- Anything you could not check, say so here. -->
