# Reversible arena

A Rust service that runs agent teams against a Postgres copy of the tau2-bench retail data, records every step together with a database snapshot, and lets you fork a run from any step.

## Run Postgres

    docker compose -f deploy/compose.yml up -d

Postgres 17 listens on `127.0.0.1:5544` (user and password `arena`). Override the test connection with `ARENA_PG_URL` (default `postgres://arena:arena@127.0.0.1:5544/postgres`).

## Run the tests

    cd core && cargo test

The tests need the Postgres above to be running.

## Attribution

The retail data in `data/retail/` (`db.json`, `tasks.json`, `policy.md`) comes from [tau2-bench](https://github.com/sierra-research/tau2-bench), MIT License, Copyright (c) 2025 Sierra Research.
