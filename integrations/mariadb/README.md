# mariadb integration

Puts a MariaDB database in the world: one database per instance on a dedicated server (`mariadb:11.8` in Docker).
Restore: exact from a save, where a save is a binary-log position and a restore rebuilds the database from the log;
approximate: no. `uses_clock`: false. Verified live: the seven `live_mariadb_*` tests ran against real MariaDB 11.8
servers in Docker (round trip, restore of a restore, dump threshold, neighbours and log rotation, accounts and server
identity, identity migration, and the `arena-part-check` conformance run).

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
- **Start:** a fresh instance is an empty database and its account, then the config's `init_sql` run
  by that account. Its `start` position (where its own changes begin) is read after the database and
  account are made and before `init_sql`, so `init_sql` is part of its changes. A restored instance's
  `start` is read after the replay (see Restore), so the replayed history is not.
- **Save:** `$ARENA_SAVES/mariadb/<save>/manifest.json` = `{server, root, segments}`: the manifest the
  instance started from, plus the segment `{db: mi_<instance>, from: start, to: now}`. A restored
  instance's saves are its parent's manifest plus its own segment. Usually no dump is made; see Dumps
  for when one is.
- **Exactness check:** `mariadb-binlog --database` picks events by the default database they ran
  under. `sql_execute` refuses `USE`, and each save scans the instance's new binlog range
  (`SHOW BINLOG EVENTS`) for DDL that names `mi_<instance>` but ran under another default database (or
  none), as a `connect` client can. If it finds any, the save fails with -32001 "changes to <instance>
  were made while another database was selected, so this save can't be exact", and no manifest is
  written. The change stays in that instance's history, so every later save of it fails the same
  way: it can't be saved exactly any more. To recover, start a new instance from its last good save
  and redo the change through `sql_execute` (or with the instance's database as the default
  database). Comments don't hide a `USE`: the check reads past `/* */`, `-- ` and `#` comments and
  into executable comments (`/*! .. */`), for `sql_execute` and `init_sql` alike.
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
- **Re-rooting:** arenad keeps every benchmark's base and gold saves (`base_*`, `gold_*`), which would hold the
  binlog from their positions for good. So `gc` rewrites the saves `keep` keeps by a pattern as full dumps once
  they alone hold more than 256 MiB of binlog (nothing else on the server needs those bytes; a save that restores
  from a dump needs no binlog). The check is tested without a server, the rewrite by
  `live_mariadb_reroots_standing_saves`.
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
accounts, position, binlog files, dump, load, apply_log, restore, chain, rebase, purge_unneeded),
`Tracked` (an instance's base, start and checked position), `Dumps` (the dumps and their gate;
`save` chains or dumps a tracked database into a save, `remove_unused`), `used_roots`,
`needed_positions`, `Manifest`, `saves_in`, `Position`, `Blobs`, `Login`, and the -32001 checks
`check_same_server`, `restore_error`, `chain_error`. The `erpnext` integration uses the same
helper on a server of its own. The SQL tool schemas, argument parsing and inspect key lookup are
shared with `postgres` in `arena_part::helpers::sql_tools`.
