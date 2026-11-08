# sculptured integration

Puts Sculptured's approvals in the world: one `sculptured serve` per instance, with changes proposed and decided
through tools. Restore: exact from a save (SQLite backups plus a data folder); approximate: no. `uses_clock`:
false. **The Sculptured program is not published yet**, so outside contributors cannot run this integration
or its live test; the rest of the repository does not need it. It needs a Sculptured with the
route for outside proposers (`POST /api/teams/{id}/changes`, Sculptured `platform` from 2026-11-07).
Verified live on 2026-11-07: `live_sculptured_roundtrip` (propose, save, decide, restore, re-raise, diff)
and `arena-part-check` (passed) ran against a local build of `sculptured` on the maintainer's machine,
by hand (no routine test runs them). The test is skipped, and says so, when no
Sculptured program is found. A save restores only with the same Sculptured build.

Sculptured's approvals in the arena world: changes proposed to a business system and the decisions on
them. No agent of Sculptured's runs for it: the tools propose through Sculptured's route for outside
systems. Each instance is a `sculptured serve` of its own; the tools reach it only through Sculptured's HTTP
API.

- **Config:** `{binary?, team?}`. `binary` is the sculptured program (default: the
  first `sculptured` on `PATH`; it is run as it is, never built). `team`
  (default `arena`; `a-z`, `0-9` and `-`) is the team whose changes the tools propose, list and decide.
- **Instance** `<instance>`: `binary serve --port 0 --data-dir <saves>/sculptured/.live/<instance>/data`,
  in a process group of its own, with a cleared environment (`HOME` is the instance's `home/`, `TMPDIR` a
  short scratch folder `/tmp/arena-sc-<pid>-<n>` for the runner's socket, `RUST_LOG=warn`, `PATH`). The
  system picks the loopback port (never 4410); it is read from `data/host-local/controld.url`. The start
  waits until serve's runner (`sculptured hostd`) is connected (`GET /api/hosts`), then makes sure the team
  exists. `.live/<instance>/run.json` records the config, the absolute program path, its
  build (`--version`), the port, the scratch folder (a fresh `/tmp/arena-sc-<16 hex>`) and the processes:
  serve's pid and group, and the runner's pid from `host-local/hostd.pid`. A restarted program still
  reaches, saves and stops the instance from that record. `.live` is a dot folder: `gc` and
  arenad's sweep leave it alone (a `live` folder would look like a save id).
- **Team:** made on the first start of a fresh server: `POST /api/teams` `{template: "blank", name}` (its id
  is its name; no agents) and `PUT /api/teams/{team}/settings` with `question_timeout_min: 1440`. A
  restored server already has it. The team has no rules, so every change waits for a person's decision.
  After a day (wall clock) unanswered, Sculptured records the change not approved, by `nobody-in-time`.
  No agent runs in the team: the tools propose as an outside system.
- **Where changes come from:** Sculptured's route for outside proposers, `POST /api/teams/{team}/changes`
  (`crates/controld/src/changes.rs`, `seat_tools/propose.rs::propose_external`). It records
  `ChangeProposed` (proposer `external.arena@<team>`) and `ChangeChecked` on the team, then asks a person
  (an `approval` question in Needs you), and records `ChangeDecided` when the answer comes, exactly as for
  an agent's `propose_change`. So:
  - `propose_change {action, summary, payload, system?, reason?}` (write): `POST /api/teams/{team}/changes`
    with the arguments and `proposer: "arena"`. Sculptured refusing the arguments (400) is the tool's
    error. It gives the change and writes `change/<id>`, and gives a world event `change_proposed` whose
    `data` is the change, so a benchmark's link can hand it to whoever decides it (a simulated person,
    say). While 8 changes wait, it refuses.
  - `list_changes {}` (read): `GET /api/teams/{team}/changes` (`crates/controld/src/changes.rs`), newest
    first. It reads every change's key.
  - `decide_change {id, approved, note?}` (write): finds the change's open question (`GET
    /api/teams/{team}/questions`, `prompt.tool_input.proposal_id`) and answers it as a person would: `POST
    /api/questions/{q}/answer` with `{kind: "allow"}`, or `{kind: "deny", reason: note}`. It then waits
    until the change shows the decision, gives the change and writes `change/<id>`. `note` goes only with
    `approved: false`. For a change proposed again after a restore (below), the question answered is its
  copy's.
  - A change is `{id, action, system, summary, payload, reason, decision}`. `decision` is `{approved, by,
    note}` (`by` is `person`, `mandate` or `nobody-in-time`), or null while it waits. Sculptured's times and
    proposing agent are left out.
  - Refusals are the tool's `error` and change nothing (`reads` and `writes` empty): bad arguments, Sculptured
    refusing a proposal, an unknown change, a change already decided, a change that can no longer be
    decided, 8 changes already waiting. An unknown tool is -32602. A server that can't be reached fails the
    call.
- **Save** `$ARENA_SAVES/sculptured/<save>/` (tool calls on the instance wait meanwhile): `data/`,
  `changes.json` and `meta.json` (`{build, aliases}`: the `sculptured --version` that wrote the store,
  and the re-raised changes). `data/` holds an SQLite online backup of each store in the data folder (`controld.sqlite`
  and the runner's `host-local/hostd.sqlite`, through rusqlite with bundled SQLite) and the kit's folder copy
  of the rest (`host-token`). The copy leaves out the stores' `-wal`, `-shm` and `-journal`,
  `host-local/hostd.pid`, `hostd.lock`, `controld.url` and sockets. `changes.json` holds the team's changes
  at the save, as the tools showed them. Saves are exact.
- **Restore** (`from`, exact only; `approximate: false`): refused with -32001 when the save's `build`
  differs from `config.binary`'s `--version`, or when more than 8 changes waited in it (they could not all
  be proposed again). Otherwise `data/` is copied into the new instance's data folder and the server starts
  as above. The seed is ignored.
- **Re-raised changes:** a restored Sculptured withdraws the questions its predecessor asked ("Sculptured
  restarted while the agent waited for an answer"), so a change that waited at the save could not be
  decided any more. The start therefore proposes every such change again (oldest first, with the same
  action, summary, payload, system and reason) and records `original → copy` in
  `.live/<instance>/aliases.json`, which saves carry in `meta.json`, so restores of restores compose (the
  latest copy counts). `list_changes`, `inspect` and `diff` show the latest copy under the original id
  and hide every copy, and `decide_change(original id)` answers the copy's question. **Sculptured's own UI
  and API show the copy under its new id**, next to the withdrawn original, which waits forever.
- **inspect / diff:** keys `change/<id>`. A live instance's changes come from its server, a save's from
  `changes.json`. `diff` lists the changes whose doc differs, decisions included.
- **stop:** kills (SIGKILL) the recorded processes: serve's group, then the runner's group (recorded, and
  the one `hostd.pid` names now; serve starts it in a session of its own). Each process is killed only
  while it is still the recorded program (`/proc/<pid>/exe`) with a whole argument that is the data
  folder or under it (the kit's `arena_part::helpers::process_group`, Linux only). Then the instance's
  folder and scratch folder are removed. Stopping an instance that does not run is a no-op. `usage` is 0:
  everything is under `ARENA_SAVES` except the scratch folders, which `stop` removes.
- **gc:** stops (as `stop`) every instance under `.live/` that this program did not start (left running
  by a program that ended without stopping it), then drops the saves `keep` does not keep.
- Not `uses_clock`. Fingerprint `{"integration": "sculptured"}`.
- Tests: `cargo test -p arena-sculptured` (unit tests, `gc` on a simulated leaked instance, and the restore
  refusals), `cargo test -p arena-sculptured -- --ignored live_` (runs real servers; skipped with a note when
  the program is missing, or set `ARENA_SCULPTURED_BINARY`).
  Conformance: `ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/sculptured`.
