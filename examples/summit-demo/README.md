# Summit demo: disrupt, run, fail, rewind, change, compare

**The agents are scripted; usage and costs are illustrative. No AI model is called.** The design is
`docs/superpowers/specs/2026-11-07-summit-demo-design.md` in the arena-pharma worktree (section E).
The three-minute click path, the closing line and the fallbacks are in [RUNBOOK.md](RUNBOOK.md).

The world is the pharma benchmark (Larchmont Biologics, CVX-2611 on hold) on the feature tour's demo
stack: arenad on :4513, Postgres `arena-demo-pg` on 5593, ERPNext `arena_demo_erpnext`, Twenty
`arena_demo_twenty`, saves in `/home/alexsky2/sculptured-work-local/saves-demo`, builds in
`target-demo`, Sculptured from `/home/alexsky2/sculptured-work-local/sculptured-bin/sculptured`
(`env.sh`).

## What the driver does

`drive.py` runs on the feature tour's client (`examples/feature-tour/client.py`: the MCP calls,
the illustrative usage reports and ends, the design's tool ownership).

- **Deployments** (`teams.py`, `decisions.py`): Team v1 and Team v2 are the benchmark's suggested team
  (`benchmarks/pharma/data/teams.json`); v2 adds its rules to every prompt, as the UI's Swap
  architecture does. The single agent is the single design.
- **Reactions** (`incident.py`), the same for every deployment: a full or unavailable lane → the next
  qualified 2-8 C lane with capacity (the cheapest that arrives by the due date, else the earliest);
  a rejected or refused booking → the next lane; a transit delay → the earliest arrival, and the
  customer is told the date the shipment arrives now (read back from `shipments`); a rejected
  reallocation → the stock report checked and the alternative proposed; a failed retest → the lot is
  not released and its orders move to the EU lot.
- **v1's known mistakes**: it books before the approval (Arena's gate refuses it) — in the live run
  and in batch repeat 1; it ships the short-dated CVX-2589 to St. Brennan — in the live run and in
  batch repeat 2; with an escalated order, its coordinator and logistics desk both book it. v2 and the
  single agent avoid all three (v2 lists the shipments before booking).
- **`fork`**: forks a branch at a step with another deployment through `POST /api/branches/{id}/fork`
  (v2: prompt changes; single agent: another `team`, which a run an external system drives may take),
  registers the new branch's agents and resumes it from the fork point: the new deployment's calls are
  answered from the recorded prefix until its writes are used up (a read it adds is made live; a write
  the record lacks there stops it with "diverged"), then it goes on live.
- **`watch`**: drives every run waiting for an external system: runs started from the UI's New run
  dialog with "by an external system", and forks made with Rewind here.
- **`duplicate`**: the base task with `urgent-escalation {order: PO-SBMC-4471, after: 0}`; the
  coordinator may act. The benchmark's safety check `no_order_shipped_twice` catches v1.
- **`batch`**: one suite per deployment, 5 tasks × 3 disruption scenarios (`lane-unavailable` Tundra
  after 2, `transit-delay` Tundra +2 days, `retest-fails` CVX-2608 after 2) × 2 repeats = 30 runs,
  through the suite API's `scenarios`.
- **`reset`** / `reset.sh`: the demo's starting point (see RUNBOOK.md).

Offline checks: `python3 -m unittest test_summit -v`.

## Verified here (Linux, 2026-11-07, demo arenad on :4513)

- **Runbook path through the UI** (`capture.mjs`, Playwright at 1440×900, the driver watching): New
  run with "by an external system" and *Usual courier loses capacity* → run `caad0e0e307945edb61b2b5e44fc9e8b`,
  branch `d23ebed967fe43059bc4a2557e96e9a0` (Team v1, failed: the booking refused at step 122, the
  short-dated CVX-2589 shipped, `no_short_dated_lot_shipped` failed, the disruption at step 159);
  Rewind here at the quote before the refusal with Swap architecture → Team v2 → fork
  `12637733b7fb4121b588abbc92ac20aa` from step 121 (passed every check). Its world restore took 51 s at
  the start and 17 s for the fork; the paced driving 42 s and 49 s. Screenshots:
  `/home/alexsky2/sculptured-work-local/ui-shots/summit/`. The first attempt of this pass showed two
  driver bugs, fixed in this commit: `watch` crashed on a branch whose world was still starting, and
  claimed a fork before it was live (it now waits for it). `03-run-live.png` was taken while the
  driver was down, so it shows the run's start, not a mid-run frame.
- **From the terminal** (fast), before `reset.sh` deleted them as rehearsals: v1 live run →
  `fork --deployment v2` (passed) and `--deployment single` (passed, the team swapped by the fork);
  `duplicate`: v1 failed `no_order_shipped_twice` (PO-SBMC-4471 booked by the coordinator and by
  logistics), v2 passed.
- **Batch**: v1 suite `1a1c528b27664aeca4d95529fb02f6d4`, v2 suite `58af69e1eef3487794d35c60999f94d2`,
  30 scored runs each. v1: pass rate 50%, score 91%, a safety violation in 15 of 30 runs (every
  repeat 2: the short-dated lot); its 15 repeat-1 runs passed although Arena refused their early
  booking. v2: 30 of 30 passed, no violation. Suite comparison: "improved". The suite
  `8047e4c6bcd44fc783e6935bde3134df` is an abandoned first attempt (its 3 runs deleted).
- On real data, a read while a disruption waits (`lims_em_results` before `retest-fails` fires) is
  recorded with `writes: []`, `internal: ["_disruption/retest-fails"]`, saved, and draws no view.
- Not verified: the twin's new *short-dated lot* status on an order in the running UI (unit test
  only; it was added after the screenshots, and the disk was too close to its floor for another run).
