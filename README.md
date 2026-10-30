# Reversible arena

A Rust service that runs agent teams against a Postgres copy of the tau2-bench retail data, records every step together with a database snapshot, and lets you fork a run from any step.

## Run Postgres

    docker compose -f deploy/compose.yml up -d

Postgres 17 listens on `127.0.0.1:5544` (user and password `arena`). Override the test connection with `ARENA_PG_URL` (default `postgres://arena:arena@127.0.0.1:5544/postgres`).

## Run the tests

    cd core && cargo test

The tests need the Postgres above to be running.

## Disk

What uses space: each branch, snapshot and gold database is a full copy of the retail database (about 21 MB), and every successful write makes a snapshot. Runs and forks add up; the arena Postgres keeps them until you delete the run. The Rust build directory `core/target` also grows; `cd core && cargo clean` frees it.

- The top bar shows `DB used / limit`. When the `branch_`, `snap_` and `gold_` databases together reach the limit, new runs and forks are refused (HTTP 507) until you delete old runs. The limit is `ARENA_MAX_DB_BYTES`, read once at startup (default 5 GiB, `5368709120`).
- Delete a run from the Runs page (Delete run, then confirm), or `curl -X DELETE http://127.0.0.1:4510/api/runs/<run_id>`. This drops all its branch and snapshot databases and its recorded events. A run with a running branch can't be deleted.
- At startup arenad drops `branch_` and `snap_` databases that no recorded branch or event refers to (left by crashes), and leftover `gold_tmp_` databases, and logs `gc: dropped N databases (X MB)`. It never touches `world_base`, `gold_*`, `arena` or other databases. Start arenad only when no test is running, since tests make unreferenced databases.
- The tests drop every database they create, even when an assertion fails. `core/scripts/leakcheck.sh` runs each test binary and fails if one leaves databases behind.

## Attribution

The retail data in `data/retail/` (`db.json`, `tasks.json`, `policy.md`) comes from [tau2-bench](https://github.com/sierra-research/tau2-bench), MIT License, Copyright (c) 2025 Sierra Research.
