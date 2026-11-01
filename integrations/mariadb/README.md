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
- **Server identity:** the first `ensure` writes a random id into `arena_meta.server`. Manifests record
  it, and a restore on a server with another id (a new volume behind the same name) is refused with
  -32001 "the MariaDB server was replaced since this save".
- **Accounts:** each instance has its own account `mi_<instance>` (random password), allowed only on its
  database: `SELECT, INSERT, UPDATE, DELETE, CREATE, DROP, ALTER, INDEX, CREATE TEMPORARY TABLES, LOCK
  TABLES`. Tools, `init_sql` and `connect` use it, so they can't turn binlogging off, purge the binlog or
  reach another database. Views, triggers, routines and events are not allowed: their `DEFINER` names an
  account that would not exist under the restored name. `stop` drops the account.
- **Start:** a fresh instance is an empty database plus the config's `init_sql`; its own changes are
  the binlog from the position after the `CREATE DATABASE`.
- **Save:** `$ARENA_SAVES/mariadb/<save>/manifest.json` = `{server, root, segments}`: the manifest the
  instance started from, plus the segment `{db: mi_<instance>, from: start, to: now}`. A restored
  instance's saves are its parent's manifest plus its own segment. No dump is made.
- **Restore** (`from`, either mode; restores are exact): a new database, the root dump if any, then for
  each segment `mariadb-binlog --database=<db> --rewrite-db=<db>-><target>` over the binlog files
  spanning it, piped into `mariadb` inside the server container. The replay runs as a temporary account
  `rp_<target>` limited to the target database (plus `BINLOG REPLAY, BINLOG ADMIN`) with
  `sql_log_bin=0`: it does not grow the binlog, and a DDL statement that names its database (`CREATE
  TABLE mi_a.x ..`) fails the restore (target dropped) instead of touching the source. The restored
  instance's own changes start at the position after the replay.
- **Dumps:** when a manifest's segments add up to more than `dump_after_bytes` of binlog (config,
  env `ARENA_MARIADB_DUMP_AFTER_BYTES`, default 64 MiB), the save writes a `mariadb-dump
  --single-transaction` of the instance instead (root, no segments) and the instance's changes start
  over from the position the dump notes (`--master-data=2`), so dump and binlog agree. Dumps are kept once by content hash in `$ARENA_SAVES/mariadb/.blobs/<sha256>.sql.gz`.
- **Tools:** none by default. `arena-mariadb --sql-tools` serves `sql_query {sql, params?}` (read) and
  `sql_execute {sql, params?}` (write), with `?` placeholders and the instance's database as the
  default database. `sql_query` takes one SELECT, WITH, SHOW, DESCRIBE or EXPLAIN and runs it in a
  `START TRANSACTION READ ONLY` that is rolled back. Keys are `<table>/<pk>` for a single-table INSERT,
  UPDATE, DELETE or SELECT that pins a one-column primary key with `=`; a REPLACE or an unpinned
  statement is `<table>/*`; several statements, a multi-table UPDATE or DELETE, or anything else is `*`.
  Write DDL without the database name: a segment holds only its own database's events, and a qualified
  name makes the save fail to restore (loudly).
- **inspect / diff:** keys `<table>/<pk>` for tables with a one-column primary key (asking for a key of
  another table is -32602); `diff` compares rows by key and, for tables without one, a hash of the whole
  table (`<table>/*`). A save is read by rebuilding it into a scratch database `mx_<pid>_<n>`.
- **drop_save** and **gc {keep}** share one path: remove the dumps no save or live instance needs, then
  `PURGE BINARY LOGS TO` the file holding the oldest position a remaining save or live instance needs
  (clamped to the files on the server). `gc` first drops the saves not kept, instance databases not
  running, scratch databases nobody reads, and their accounts. A manifest that can't be read stops
  both with an error rather than purging on a guess. Use one `ARENA_SAVES` per
  server: saves under another root are not seen and their binlog may be purged.
- **usage** is the binlog files plus the `mi_*` and `mx_*` databases (saves and dumps live under
  `ARENA_SAVES`, which arenad counts).
- **connect** gives `{url}` (`mysql://mi_<instance>:<password, percent-encoded>@127.0.0.1:<port>/mi_<instance>`),
  the instance's own account.
- **fingerprint** `{server_version}` is the image's version, `11.8`.
- **Conformance:** `check/arena-package.toml` is the package `mariadb-check`, this program with
  `--sql-tools`. `cargo test -p arena-mariadb -- --ignored live_` runs it (and the other live tests)
  against a server of its own, removed afterwards.

The helper is `arena_part::helpers::mariadb` (feature `mariadb`): `Server` (ensure, identity,
accounts, position, binlog files, dump, load, apply_log, restore), `Manifest`, `Position`, `Blobs`,
`Login`. The SQL tool schemas, argument parsing and inspect key lookup are shared with `postgres` in
`arena_part::helpers::sql_tools`.
