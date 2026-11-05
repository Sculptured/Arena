# twenty-crm integration

Puts [Twenty](https://twenty.com), an open-source CRM, in the world: one Twenty server per instance (`twentycrm/twenty:v2.51.0`
in Docker), each with its own Redis, on one database of one Postgres server. Restore: exact from a save (`CREATE
DATABASE .. TEMPLATE`, the kit's Postgres helper); approximate: no. `uses_clock`: false.

**Verified:** `live_twenty_roundtrip` ran against that real image in Docker, on Linux: find, write, inspect, save, change,
restore, a restarted program, `gc`, and the `arena-part-check` conformance run (all steps passed, replay skipped: it
restores from saves). The test took about 100 s, of which the first start, making the base, about 60 s. A restore took about 9 s; a save
about 0.1 s and 20 MB of database, on the database server's Docker volume. A server holds about 1 GB of memory. **Not run:** Twenty's web UI against a restored instance, and more than two instances at once.

- **Database server:** compose project `arena_twenty` from `compose.yml`: `postgres:17-alpine` (never pulled), volume
  `arena-twenty-db`, container `arena-twenty-db`, on `127.0.0.1:5546`, network `arena-twenty`. It comes up on the first
  start or the first call that needs it, and stays up. Env `ARENA_TWENTY_PROJECT`, `ARENA_TWENTY_DB_PORT` and
  `ARENA_TWENTY_IMAGE` change it, or the first start's config (`project`, `db_port`), since arenad runs programs with a cleared
  environment. A later start whose config names another server is -32602.
- **Base** (made once, kept as the database `tb_base`, with its sign-in in `$ARENA_SAVES/twenty-crm/.base/api.json`): a server
  on an empty database runs the image's migrations, the app's sign-up makes the workspace and an Admin API key, the server
  is removed, and the demo records a new workspace comes with are emptied. A base without its sign-in file is made again.
- **Instance** `<id>` (at most 60 characters) is the database `ti_<id>` copied from the base or a save, and compose project
  `arena_twenty_<id>` from `instance.yml`: the server, on a free port on 127.0.0.1, without migrations, and a Redis with no
  persistence. Every copy of a database has the same workspace and API key. `.live/<id>.json` records its port and server, so
  a restarted program can still use, save and stop it. `stop` removes the project and the database.
- **State changes only through tool calls:** there is no worker service and no cron registration, so nothing writes in the
  background. What a worker would do is not done: timeline activity rows and webhooks.
- **Save** `ts_<save>` is a copy of the instance's database. Postgres can't copy a database with connections open, so the
  server's are closed for the copy, and its pools open new ones on the next request. `dir` is ignored: saves
  are kept by Postgres, as in the `postgres` integration. The server's Redis holds only caches and is not saved.
- **Tools** (REST on the instance's port with the API key; a refusal or bad argument is the tool's error, Twenty not answering
  fails the call): `crm_find_account {name?}` (with its contacts and opportunities), `crm_find_contact {name?, email?,
  account_id?}`, `crm_find_opportunities {name?, account_id?}`, each also taking `limit` (default 60) and `cursor` and
  answering a page: the records as Twenty's REST API gives them (depth 1), with `id`, `name`, `account_id`, `close_date`,
  `amount` and the like set over them, plus `total` and, when there are more, `next_cursor`; `crm_log_note {account_id, title, body}`,
  `crm_update_opportunity {id, stage?, close_date?}`, `crm_create_task {title, body?, due?, account_id?}`. Reads are
  `<table>/<id>` keys. An account is Twenty's `company`.
- **inspect / diff** read the database directly, so a save needs no server. Keys are `<table>/<id>` for `company`, `person`,
  `opportunity`, `note`, `noteTarget`, `task` and `taskTarget`; the documents are the rows, plus `accounts` (names) on a note
  or task and `account` on an opportunity or person. Other tables are copied by a save but not compared. With keys, a key
  that isn't there is null.
- **connect** gives `{url, api_key, admin_email, admin_password}`, for a benchmark's seed and for signing in to the app.
- **usage** is the size of the `ti_*` and `ts_*` databases; 0 before this program has needed the server. **gc {keep}** drops the
  saves `keep` doesn't name (an entry ending in `*` is a prefix) and the databases of instances not running; never the base.
- **Conformance:** `check/arena-package.toml` (`twenty-crm-check`, the same program). `cargo test -p arena-twenty-crm --
  --ignored live_ --nocapture` runs the round trip and the check against a server of its own and removes it when it ends.
  It needs the three images above present; none is pulled.
