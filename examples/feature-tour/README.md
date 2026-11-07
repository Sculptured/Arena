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

The checked-in `env.sh` contains the exact paths and ports, and sets a 12 GiB save budget
(`ARENA_MAX_DB_BYTES=12884901888`) for the full recording set. This budget is separate from
the driver's 12 GiB minimum free-space guard. `ARENA_RUN_ERPNEXT` and
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
By default each invocation creates new suites. `--resume --output <same file>` explicitly
continues pending slots from the recorded suite IDs and refuses active, missing or errored runs.
Completed runs are never replayed or overwritten.
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
- Driver: 13 offline tests pass, including explicit suite resumption and bounded empty-read handling.
- `arenad base` succeeds with `/home/alexsky2/sculptured-work-local/sculptured-bin/sculptured`
  (the authorized standalone build). The old installed binary lacked the changes API. The
  retry also required removing an empty `sculptured/base_pharma_0_1_0_approvals` directory left
  by the failed attempt; no valid save was removed.
- The first pharma live test passed all 13 checks on the base incident, then failed restoring
  the next scenario: Sculptured had resumed the saved Practice team, so starting it again
  returned HTTP 409. Commit `988837c` stops that restored generation before restarting it.
  The Sculptured live round-trip test, 14 offline tests, formatting and strict Clippy pass.

The exact capture environment is `env.sh`; it puts the authorized Sculptured build first on
`PATH` as well as setting `ARENA_SCULPTURED_BINARY`. Keep that build available for restores.

The pharma live test passed after the restore fix (601.37 s): both correct paths passed all
13 checks, and the unsafe path failed the intended checks. It used its own temporary stacks;
arenad was stopped during the test.

One initial v2 attempt (`137f3ba31d2a48d394e67d1984c9528c`) stopped on carrier-capacity repeat 2
when the ERP's filtered work-order query returned an empty list. Its failed branch
`36a70a4b503d44b6b7089cf32029b0af` is retained. Inspection of its saved base found
`Work Order/MFG-WO-2026-00002` with `planned_batch=CVX-2615`; the cause of the empty live query
is unresolved. The driver now makes at most three recorded reads before failing clearly.
A fresh v2 suite is used for the final comparison; the interrupted suite is diagnostic evidence,
not a passing result. Its manifest is `/home/alexsky2/sculptured-work-local/demo-interrupted-results.json`.
The server was restarted between attempts with the larger save budget, before the default
5 GiB budget was reached. No recorded outcome was changed.

## The video

Made on November 6, 2026 from the recorded runs above: team v1 (10 of 10 runs), team v2 (10 of 10)
and one paced live run of team v1 on `cvx-2611-em-excursion`. The single-agent suite was not run for
the video. Sculptured screens come from a throwaway instance (`--port 4620`, its own data dir and
HOME, practice agents) with `SCULPTURED_ARENA_URL=http://127.0.0.1:4513`.

- Stills: `examples/factory-showcase/capture/stills.mjs` (focus on a panel, 2x); the live process
  view: `capture/record.mjs` during the live run, cropped to the diagram with ffmpeg.
- Narration: `script.yaml` read by Kokoro (`video/narrate_kokoro.py`, voice `af_heart`), checked
  against a faster-whisper transcript.
- Edit: `edit.yaml` with `video/build.py`, then `ffmpeg -af loudnorm=I=-16:TP=-1.5:LRA=11`.
