# postgres integration

Puts a PostgreSQL database in the world: one database per instance, on a Postgres server you run. Restore: exact from a save
(`CREATE DATABASE .. TEMPLATE`); approximate: no. `uses_clock`: false. Verified live: its tests (`cargo test -p
arena-postgres`) run against the `postgres:17-alpine` server of `deploy/compose.yml`, including the `arena-part-check`
conformance run; the retail benchmark runs on it through arenad. These tests are not marked `live_`, because Shakedown
needs that Postgres anyway.

Each instance is a database `pi_<instance>`; each save is a copy of it, `ps_<save>`, made with
`CREATE DATABASE .. TEMPLATE`. Saves are kept by Postgres itself (the `dir` of `save` is ignored) and
restore exactly. `stop` drops the instance's database, so save first. `usage` counts the sizes of
`pi_*` and `ps_*` databases, and `gc {keep}` drops saves not kept (an entry ending in `*` keeps every save id with that prefix)
and instances not running.

- Start with config `init_sql` runs it in a fresh empty database; start `from` a save copies it.
- `inspect` keys are `<table>/<primary key>` (tables with a one-column primary key); `diff` compares all rows.
- `connect` gives `{url}` for benchmarks to use.
- The server is `ARENA_PG_URL` (default `postgres://arena:arena@127.0.0.1:5544/postgres`).
- No tools by default, so agents get none from this package. `arena-postgres --sql-tools` lists and serves
  `sql_query` and `sql_execute`.
- Old runs: an id naming an existing `snap_*`, `branch_*`, `gold_*` database or `world_base` is used as it is;
  `usage` and `gc` also cover `snap_*` and `branch_*`, and `drop_save` drops a `snap_*` or `branch_*` database
  (never `gold_*` or `world_base`).
- Namespace: env `ARENA_PG_NS`, or `?arena_ns=<ns>` on `ARENA_PG_URL`, puts `<ns>` after `pi_`/`ps_` and turns
  legacy names off, so tests never touch anyone else's databases.
- Conformance: `check/arena-package.toml` is the package `postgres-check`, this program with `--sql-tools` and
  example calls: `arena-part-check integrations/postgres/check --config check.json`.
