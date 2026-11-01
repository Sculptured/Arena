# mariadb integration

Each instance is a database `mi_<instance>` (ids of at most 61 characters) on a dedicated MariaDB
server with row-based binary logging. A save does not copy the database: it records how to rebuild
it, so saving is a few milliseconds and the user's words hold: save the changes, apply them like a
database.

- **Server:** container `arena-mariadb` (image `mariadb:11.8`, never pulled), volume
  `arena_mariadb_data`, on `127.0.0.1:3307`, root password `arena`, started on the first `start` if it
  is not running. Flags: `--log-bin=binlog --binlog-format=ROW --binlog-row-image=FULL --server-id=1
  --binlog_expire_logs_seconds=0 --max_binlog_size=64M`. The config keys `container`, `port`,
  `volume`, `root_password` (env fallbacks `ARENA_MARIADB_CONTAINER`, `_PORT`, `_VOLUME`, `_PASSWORD`)
  pick another server. Containers are made with the label `arena.part=mariadb`, and a container of
  the configured name without that label is refused, so no other MariaDB is ever touched.
- **Start:** a fresh instance is an empty database plus the config's `init_sql`; its own changes are
  the binlog from the position after the `CREATE DATABASE`.
- **Save:** `$ARENA_SAVES/mariadb/<save>/manifest.json` = `{server, root, segments}`: the manifest the
  instance started from, plus the segment `{db: mi_<instance>, from: start, to: now}`. A restored
  instance's saves are its parent's manifest plus its own segment. No dump is made.
- **Restore** (`from`, either mode; restores are exact): a new database, the root dump if any, then for
  each segment `mariadb-binlog --database=<db> --rewrite-db=<db>-><target>` over the binlog files
  spanning it, piped into `mariadb` inside the server container. The replay runs with
  `sql_log_bin=0`, so it does not grow the binlog, and the restored instance's own changes start at the
  position after it.
- **Dumps:** when a manifest's segments add up to more than `dump_after_bytes` of binlog (config,
  env `ARENA_MARIADB_DUMP_AFTER_BYTES`, default 64 MiB), the save writes a `mariadb-dump
  --single-transaction` of the instance instead (root, no segments) and the instance's changes start
  over from there. Dumps are kept once by content hash in `$ARENA_SAVES/mariadb/.blobs/<sha256>.sql.gz`.
- **Tools:** none by default. `arena-mariadb --sql-tools` serves `sql_query {sql, params?}` (read) and
  `sql_execute {sql, params?}` (write), with `?` placeholders and the instance's database as the
  default database. Keys are `<table>/<pk>` for a single-table INSERT, UPDATE, DELETE or SELECT that
  pins a one-column primary key with `=`, else `<table>/*`, else `*`. Statements on another database
  (`db.table`) are not captured by saves: a segment holds only its own database's events.
- **inspect / diff:** keys `<table>/<pk>` for tables with a one-column primary key (asking for a key of
  another table is -32602); `diff` compares rows by key and, for tables without one, a hash of the whole
  table (`<table>/*`). A save is read by rebuilding it into a scratch database `mx_<pid>_<n>`.
- **drop_save** removes the save's folder and every dump no save or live instance needs. **gc {keep}**
  drops saves not kept, instance databases not running, unused dumps, and `PURGE BINARY LOGS TO` the
  file holding the oldest position a kept save or live instance needs. Use one `ARENA_SAVES` per
  server: saves under another root are not seen and their binlog may be purged.
- **usage** is the binlog files plus the `mi_*` and `mx_*` databases (saves and dumps live under
  `ARENA_SAVES`, which arenad counts).
- **connect** gives `{url}` (`mysql://root:<password>@127.0.0.1:<port>/mi_<instance>`); writes through it
  are captured when they use that default database.
- **fingerprint** `{server_version}` is the image's version, `11.8`.
- **Conformance:** `check/arena-package.toml` is the package `mariadb-check`, this program with
  `--sql-tools`. `cargo test -p arena-mariadb -- --ignored live_` runs it (and the other live tests)
  against a server of its own, removed afterwards.

The helper is `arena_part::helpers::mariadb` (feature `mariadb`): `Server` (ensure, position, binlog
files, dump, load, apply_log, restore), `Manifest`, `Position`, `Blobs`.
