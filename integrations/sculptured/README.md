# sculptured integration

Sculptured's approvals in the arena world: changes proposed to a business system and the decisions on
them. Each instance is a `sculptured serve` of its own; the tools reach it only through Sculptured's HTTP
API.

- **Config:** `{binary?, team?}`. `binary` is the sculptured program (default
  `~/MLSys/Sculptured/sculptured/target/release/sculptured`; it is run as it is, never built). `team`
  (default `arena`; `a-z`, `0-9` and `-`) is the team whose changes the tools propose, list and decide.
- **Instance** `<instance>`: `binary serve --port 0 --data-dir <saves>/sculptured/.live/<instance>/data`,
  in a process group of its own, with a cleared environment (`HOME` is the instance's `home/`, `TMPDIR` a
  short scratch folder `/tmp/arena-sc-<pid>-<n>` for the runner's socket, `RUST_LOG=warn`, `PATH`). The
  system picks the loopback port (never 4410); it is read from `data/host-local/controld.url`. The start
  waits until serve's runner (`sculptured hostd`) is connected (`GET /api/hosts`), then makes sure the team
  exists and runs. `.live/<instance>/run.json` records the config, port and scratch folder, so a
  restarted program still reaches, saves and stops the instance. `.live` is a dot folder: `gc` and
  arenad's sweep leave it alone (a `live` folder would look like a save id).
- **Team:** made on the first start of a fresh server: `POST /api/teams` `{template: "blank", practice:
  true, name}` (its id is its name), `PUT /api/teams/{team}` with 8 Practice agents `p1`..`p8` (the stub
  driver, no model), and `PUT /api/teams/{team}/settings` with `question_timeout_min: 1440`. Then `POST
  /api/teams/{team}/start`, and the start waits until every agent runs (`GET /api/rigs/{team}`). A
  restored server lost its agents' processes, so its start starts the team again. The team has no rules,
  so every change waits for a person's decision. After a day (wall clock) unanswered, Sculptured records
  the change not approved, by `nobody-in-time`.
- **Where changes come from:** Sculptured has no HTTP route that proposes or decides a change. A change
  is made only by its agent tool `propose_change` (`crates/controld/src/seat_tools/propose.rs`). The tool
  records `ChangeProposed` and `ChangeChecked` on the team, then asks a person (an `approval` question
  whose waiter is the agent's call), and records `ChangeDecided` when the answer comes. So:
  - `propose_change {action, summary, payload, system?, reason?}` (write): finds an agent with no question
    open, sends it `propose_change:<args>` (`POST /api/seats/{seat}/send`, intent `instruct`), and reads its
    events (`GET /api/seats/{seat}/events?after=<seq>`) until Sculptured asks about the change (the
    `question-raised` event gives the change's id) or refuses the arguments (the reply of the turn that
    took the message). It gives the change and writes `change/<id>`. While 8 changes wait, it refuses.
  - `list_changes {}` (read): `GET /api/teams/{team}/changes` (`crates/controld/src/changes.rs`), newest
    first. It reads every change's key.
  - `decide_change {id, approved, note?}` (write): finds the change's open question (`GET
    /api/teams/{team}/questions`, `prompt.tool_input.proposal_id`) and answers it as a person would: `POST
    /api/questions/{q}/answer` with `{kind: "allow"}`, or `{kind: "deny", reason: note}`. It then waits
    until the change shows the decision, gives the change and writes `change/<id>`. `note` goes only with
    `approved: false`.
  - A change is `{id, action, system, summary, payload, reason, decision}`. `decision` is `{approved, by,
    note}` (`by` is `person`, `mandate` or `nobody-in-time`), or null while it waits. Sculptured's times and
    proposing agent are left out.
  - Refusals are the tool's `error` and change nothing (`reads` and `writes` empty): bad arguments, Sculptured
    refusing a proposal, an unknown change, a change already decided, a change that can no longer be
    decided, 8 changes already waiting. An unknown tool is -32602. A server that can't be reached fails the
    call.
- **Save** `$ARENA_SAVES/sculptured/<save>/` (tool calls on the instance wait meanwhile): `data/` and
  `changes.json`. `data/` holds an SQLite online backup of each store in the data folder (`controld.sqlite`
  and the runner's `host-local/hostd.sqlite`, through rusqlite with bundled SQLite) and the kit's folder copy
  of the rest (`host-token`). The copy leaves out the stores' `-wal`, `-shm` and `-journal`,
  `host-local/hostd.pid`, `hostd.lock`, `controld.url` and sockets. `changes.json` holds the team's changes
  at the save. Saves are exact.
- **Restore** (`from`, exact only; `approximate: false`): `data/` is copied into the new instance's data
  folder, then the server starts as above. The seed is ignored. A change that waited for a decision when
  it was saved waits again, but **it can no longer be decided**. Its question's waiter was the agent's
  call in the old server, so the restored Sculptured withdraws the question at start ("Sculptured
  restarted while the agent waited for an answer"), just as when Sculptured itself restarts.
  `decide_change` then refuses and says to propose the change again.
- **inspect / diff:** keys `change/<id>`. A live instance's changes come from its server, a save's from
  `changes.json`. `diff` lists the changes whose doc differs, decisions included.
- **stop:** kills (SIGKILL) the process group of every process whose command line names the instance's
  data folder: serve, then its runner, which serve starts in a session of its own. The helper is the
  kit's `arena_part::helpers::process_group::kill_matching`. Then the instance's folder and scratch
  folder are removed. `usage` is 0: everything is under `ARENA_SAVES` except the scratch folders, which
  `stop` removes.
- Not `uses_clock`. Fingerprint `{"integration": "sculptured"}`.
- Tests: `cargo test -p arena-sculptured` (unit), `cargo test -p arena-sculptured -- --ignored live_` (runs
  real servers; skipped with a note when the program is missing, or set `ARENA_SCULPTURED_BINARY`).
  Conformance: `ARENA_BIN_DIR=core/target/debug core/target/debug/arena-part-check integrations/sculptured`.
