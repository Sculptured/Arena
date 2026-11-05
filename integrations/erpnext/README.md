# erpnext integration

Puts an ERPNext site in the world: one site per instance on one ERPNext install (`frappe/erpnext:v16.53.0` in Docker),
with its own MariaDB server. Restore: exact from a save (a binary-log position plus the site's files); approximate:
no. `uses_clock`: false. Verified live: `live_erpnext_roundtrip` ran against that real install in Docker (insert,
save, change, restore, diff, a restarted program, and the `arena-part-check` conformance run). First start takes
about 90 s, to build the base site.

ERP is a wrapper on a database, so ERPNext saves are the mariadb integration's binlog saves plus the
site's files. One ERPNext install runs; each instance is a site on it whose database is restored from
a save, and a save records how to rebuild that database (see `integrations/mariadb/README.md`, whose
helper this integration uses: `arena_part::helpers::mariadb`).

- **Install:** compose project `arena_erpnext` from `compose.yml` (adapted from the factory's
  `erp/compose.yml`): image `frappe/erpnext:v16.53.0` (never pulled), backend, one worker for all
  queues, websocket, frontend on `127.0.0.1:8180`, its own `redis-cache` and `redis-queue`. There is
  no `db` service, no `scheduler` and no `configurator`. It comes up on the first `start` (or the
  first call that needs it).
- **Database server:** a MariaDB server made by the mariadb helper (labelled `arena.part=mariadb`, a
  foreign container of that name is refused), separate from the mariadb integration's:
  container `arena-erpnext-db`, volume `arena_erpnext_db`, `127.0.0.1:3308`, root password `arena`,
  with Frappe's flags (`--character-set-server=utf8mb4 --collation-server=utf8mb4_unicode_ci
  --skip-character-set-client-handshake`) after the binlog flags. It is connected to the project's
  network with alias `db` (`docker network connect --alias db`). A server of its own means the
  mariadb integration's binlog purges never remove binlog an ERPNext save needs, and the reverse.
- **Settings:** the install's settings come from env (`ARENA_ERPNEXT_DB_CONTAINER`, `_DB_PORT`,
  `_DB_VOLUME`, `_DB_PASSWORD`, `_PROJECT`, `_PORT`, `_IMAGE`, `_COMPOSE`) else the defaults above, and
  the config of the first start may override them with `db_container`, `db_port`, `db_volume`,
  `db_root_password`, `project`, `http_port` (arenad runs programs with a cleared environment). One
  install per program: a later start whose config names another is -32602. Per instance:
  `diff_doctypes` (default `Item, Supplier, Customer, Purchase Order, Sales Order, Work Order,
  Journal Entry`) and `dump_after_bytes` (default 64 MiB, as mariadb).
- **Sites folder:** the bench's `sites` is `$ARENA_SAVES/erpnext/.sites` (a dot folder: arenad's
  sweep skips it), bind-mounted into the containers, so a site's files are copied with the folder
  helper. The integration writes `common_site_config.json` (db `db:3306`, the two Redis) and
  `apps.txt`. The image runs as uid 1000, so the folder must belong to uid 1000 (checked: else
  `start` fails saying so).
- **Base** (made once, on the first start, and kept in `$ARENA_SAVES/erpnext/.base/`): the helper
  makes a database `erpnext_base` and its least-privilege account, `bench new-site
  arena-base.localhost --no-setup-db --install-app erpnext` installs into them, `generate_keys` makes an
  Administrator API key, and the setup wizard runs (company `Arena Co`, abbreviation `AC`, USD,
  United States, UTC, standard chart of accounts, fiscal year 2026), so items, orders and entries
  can be made. After its background jobs, the base is saved: a dump of the database (a dump-rooted
  manifest, so it restores on any server), the site's files (`files/`), its config without the
  database keys (`site_config.json`, which keeps `encryption_key`: the stored API secret needs it) and
  the key and Administrator password (`api.json`). The manifest is written last; a base without one is
  made again. Then the base site, its database and account are removed, and the install's binlog is
  purged. When the install comes up, the base's manifest is labelled with the running server's
  identity (it has no segments, so it restores anywhere): a recreated or reconfigured server keeps
  working, and the saves chained from it name the server their binlog is on. First-time setup measured at about 90 s for the base (plus the stack coming up).
- **Grants:** the install and every site run with the helper's `DB_GRANTS` (`SELECT, INSERT,
  UPDATE, DELETE, CREATE, DROP, ALTER, INDEX, CREATE TEMPORARY TABLES, LOCK TABLES` on the site's own
  database). That was enough for `new-site` with ERPNext (758 tables and 5 sequences; no views,
  triggers or routines), so no other grant set is needed.
- **Instance** `<instance>` (ids of at most 61 characters) is the site `<instance>.localhost` on database
  `eb_<instance>`: restored from the base's or a save's manifest (as mariadb restores, by a replay
  account limited to it), with its own account `eb_<instance>` (random password), its folder made from
  the save's files plus `logs/`, `locks/` and a `site_config.json` = the base's shared config, its
  database and account, and `pause_scheduler: 1`. Its Redis cache keys (`eb_<instance>|*`) are
  deleted, and the start waits until the site answers an authenticated request (Frappe keeps a site's
  config in each worker for up to 60 s, so a site made again under a recent name may take that long).
  `stop` removes the folder, account and database. A start marks the site's database busy until the
  instance is live, so `gc` leaves it alone; starting an id that runs or is starting is -32602.
- **State changes only through tool calls:** there is no scheduler service, and every site has
  `pause_scheduler`. Jobs that tool calls enqueue run on the worker.
- **Save** `$ARENA_SAVES/erpnext/<save>/`: first the site's background jobs are drained (Frappe's `RQ
  Job` list for the site, queued or started, polled every 250 ms; after 60 s the save fails with
  -32001 "background jobs still running for <site>"). Then `files/public` and `files/private` (copies of
  `sites/<site>/public/files` and `private/files`, hard-linked to the previous save's copy when
  unchanged), `site.json` (the instance's `diff_doctypes`) and last `manifest.json`: the instance's base
  plus its own binlog segment, or a dump when restoring would read more than `dump_after_bytes` of
  binlog (exactly the mariadb integration's rule, via the helper's `Tracked`, `Server::chain` and
  `Server::rebase`). Saves are exact.
- **Restore** (`from`, either mode; exact only, `approximate: false`): a new instance from the save's
  manifest and files, as above. A save made on another server (or a replaced one) is -32001.
- **Tools** (REST on `127.0.0.1:8180` with the Administrator's key and `Host: <site>`):
  `get_doc {doctype, name}` (read), `list_docs {doctype, filters?, fields?, limit?, start?, order_by?}` (read; `name` is
  always among the fields; `limit` default 20, 0 for all; `start` skips that many, for the next page;
  `order_by` is Frappe's), `insert_doc {doc}` (write; `doc` has its
  `doctype`), `update_doc {doctype, name, fields}` (write), `submit_doc {doctype, name}` (write; sets
  `docstatus` 1, which Frappe runs as a submit). Keys are `<Doctype>/<name>`. Frappe refusing a call (a
  missing document, a failed validation) is the tool's error, with Frappe's message; bad arguments are
  tool errors too. Only ERPNext not answering (connection refused, HTTP 502 to 504) fails the call.
- **inspect / diff** read through REST: an instance's site in place; a save through a scratch site
  `view-<pid>-<n>.localhost` on database `ev_<pid>_<n>`, made from it and removed after. `inspect`
  with no keys gives every document of the target's `diff_doctypes`; `diff` lists all fields of every
  document of both targets' `diff_doctypes` and compares them by key. A target diffed with itself is
  not read. In `inspect`, a key Frappe answers 404 for, or of a doctype it doesn't have, is null; other refusals are errors.
- **drop_save** and **gc {keep}**: as mariadb (remove the dumps no save, live instance or the base
  needs; purge the binlog before the oldest position still needed), and when nothing needs any
  position and no site is being made, the binlog is rotated first so all of it goes. `gc` also drops
  the saves not kept, the databases and accounts of sites not live and not being read, and their
  folders.
- **Re-rooting:** as in the mariadb integration, `gc` rewrites the saves `keep` keeps by a pattern (`base_*`,
  `gold_*`) as full dumps once they alone hold more than 256 MiB of binlog; their files stay as they are. It shares
  mariadb's code (the kit's `Dumps::reroot_standing`) and has not been run live on this integration.
- **usage** is this server's binlog plus the `eb_*` and `ev_*` databases. The mariadb integration does
  not count this server, so nothing is counted twice. The sites folder, saves, dumps and the base are
  under `ARENA_SAVES`, which arenad counts; the server's own data volume and the Redis volumes are not
  counted.
- **connect** gives `{url, site, api_key, api_secret, admin_password}`: send `Host: <site>` (or log in to
  the web pages as Administrator).
- **fingerprint** `{image, frappe, erpnext}`: the image tag and the app versions `bench version` prints
  (run once per program in a throwaway container).
- **Conformance:** `check/arena-package.toml` (`erpnext-check`, the same program; its examples insert a
  ToDo and an Item and read them). `cargo test -p arena-erpnext -- --ignored live_ --nocapture` runs
  the round trip and the check against a project (`arena_erpnext_t<pid>`, on a free port) and a
  server of its own, and removes them (`docker compose down -v`, the server's container and volume,
  the saves and sites) when it ends.
