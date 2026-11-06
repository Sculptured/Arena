# Pharma feature-tour recordings

**The agents are scripted; usage numbers are illustrative. No AI model is called.**
The fictional pharma incident runs against real ERPNext, Twenty and Sculptured approvals,
with simulated LIMS, carriers, QA head and supply-chain director. These recordings illustrate
Arena's recording and comparison features; they are not measurements of model reliability.

`behaviour.py` fixes every scenario/repeat decision. V1 attempts three unapproved bookings
(refused by Arena) and ships one short-dated lot in base repeat 2. V2 corrects both. The single
agent follows the safe path with extra reads and 0.35 seconds of added delay per call. Reported
tokens grow with each agent's context; costs use illustrative $3/M input and $15/M output.
Wall time is measured by Arena and includes deliberate script pacing.

The roles and allowed tool groups come from `benchmarks/pharma/data/teams.json`. The driver
checks tool ownership before each call. Team members exchange recorded `message`/`inbox`
calls; only the records analyst queries the raw ERP and CRM. Approval decisions are made by
the benchmark's simulated people through world links, never by the agents.

## Recreate on this machine

Run from `/home/alexsky2/MLSys/Sculptured/arena-demo`, branch `feature-tour`.
Check `df -h /` frequently; stop below 12 GiB free. Do not run tests while arenad is serving.
The driver checks this disk floor before every HTTP request.

```bash
source examples/feature-tour/env.sh
# Only if this exact demo container does not already exist:
docker run -d --name arena-demo-pg -e POSTGRES_USER=arena -e POSTGRES_PASSWORD=arena \
  -p 127.0.0.1:5593:5432 postgres:17-alpine
nice cargo build -p arena-pharma -p arena-erpnext -p arena-twenty-crm -p arena-sculptured
nice cargo build --manifest-path core/Cargo.toml -p arenad
nice "$ARENA_BIN_DIR/arenad" base
```

The checked-in `env.sh` contains the exact paths and ports. `ARENA_RUN_ERPNEXT` and
`ARENA_RUN_TWENTY_CRM` pass stack settings through Arena's cleared subprocess environment.
The demo uses `arena_demo_erpnext`, `arena-demo-erpnext-db` (3393), `arena_demo_twenty`
(database 5594), saves in `/home/alexsky2/sculptured-work-local/saves-demo`, and HTTP port
4513. It never uses the default ERP/CRM stacks or port 5544.

Before capture, build the UI and start the server:

```bash
(cd ui && nice npm run build)
source examples/feature-tour/env.sh
export ARENA_UI_DIST=/home/alexsky2/MLSys/Sculptured/arena-demo/ui/dist
nice "$ARENA_BIN_DIR/arenad"
```

In another terminal:

```bash
nice python3 examples/feature-tour/drive.py suites \
  --output /home/alexsky2/sculptured-work-local/demo-results.json
nice python3 examples/feature-tour/drive.py live --pace 1.5 --deployment v1 \
  --task cvx-2611-em-excursion \
  --output /home/alexsky2/sculptured-work-local/demo-live.json
```

`suites` creates 10 v1 runs, 10 v2 runs and 5 single-agent runs, claiming them sequentially
through the external suite API. It saves IDs immediately, then reports and the v1→v2 comparison.
Each invocation creates new suites; it does not silently resume or overwrite server records.
A tool failure stops the driver after ending/scoring the current branch. A disk-floor failure
halts HTTP work; stop arenad before restarting after freeing space.

## Checks

```bash
python3 -m unittest discover -s examples/feature-tour -v
source examples/feature-tour/env.sh
# Existing core tests expect the default retail benchmark.
(unset ARENA_BENCHMARK ARENA_RUN_ERPNEXT ARENA_RUN_TWENTY_CRM
 nice cargo test --manifest-path core/Cargo.toml -p arenad)
nice cargo test -p arena-pharma
# The live test creates and cleans its own isolated temporary stacks; do not override them.
(unset ARENA_RUN_ERPNEXT ARENA_RUN_TWENTY_CRM
 nice cargo test --manifest-path core/Cargo.toml -p arenad --test pharma -- --ignored live_ --nocapture)
```

Keep the Postgres container and saves for capture. Stop arenad with Ctrl-C, then stop only the
demo ERPNext and Twenty containers, preserving their volumes. Never stop or remove the shared
`arena_erpnext-*`, `arena-erpnext-db`, `arena-twenty-db`, or `deploy-pg-1` services.

## Verified here (Linux, 2026-11-06)

- Merge `3713cb6`: formatting and strict Clippy (`--workspace --all-targets -- -D warnings`) in
  both workspaces; `cargo test -p arenad` (250 passed, 6 ignored) and `cargo test -p arena-pharma`
  (25 passed, 1 ignored). The first core attempt used the wrong benchmark environment; the
  clean rerun passed.
- The new run-name API test passes; core strict Clippy passes after that change.
- UI: 68 tests pass and production build passes. The display-name change has not been checked
  in a running browser.
- Python kit: 43 passed; pharma LIMS: 7 passed, 1 skipped; logistics: 8 passed, 1 skipped.
- Driver: 9 offline tests pass. The driver has **not** completed a live run.
- `arenad base` and the requested `live_pharma_right_paths_pass_and_wrong_path_fails` both
  failed while saving approvals: the installed `/home/alexsky2/.local/bin/sculptured` returns
  HTTP 404 for `/api/teams/arena/changes`. It lacks the approvals implementation required by
  `integrations/sculptured`. No model was called.

**Blocked:** a compatible standalone Sculptured binary is needed. The live-test helper supports
`ARENA_SCULPTURED_BINARY`; for arenad, put that binary's directory first on `PATH` (named
`sculptured`). Do not use another worktree's binary without authorization. Retry the base and
live test, then the suites. There are no demo suite IDs or scored branch IDs yet; no suite
comparison has been verified. The isolated Postgres and partially initialized saves are retained.

