# factory

A benchmark of a whole incident across four systems. A press's spindle bearing fails. A team has to find out what
failed from the evidence, source the right part with approval, get it on a truck, schedule the repair and tell the
customers. Its world has four parts:

| Part | Integration | What it holds |
|---|---|---|
| `erp` | `erpnext` | Items, suppliers, customers, the press's service manual, purchase orders, to-dos |
| `hub` | `ridgeline-hub` | The simulated suppliers, carriers and customers |
| `plant` | `isaac-sim-standin`, or `isaac-sim` with `--backend isaac` | The plant, on the arena clock |
| `approvals` | `sculptured` | Changes proposed and decided |

**The factory example this benchmark reads is not public yet, and neither is the Sculptured program.** The hub and
the plant wrap the example, the ERP's starting records and the expected answers come from its data file
(`Sculptured-Examples/factory/data/ridgeline.json`, next to the arena checkout, or `--data <path>`), and the
`approvals` part runs the Sculptured program. Outside contributors can read and change the code, and its unit
tests run, but the world can't start without them: `seed` and `score` say so, and the live test skips.

It is `opt_in`, so arenad's default stays the retail example; choose it with `ARENA_BENCHMARK=factory`.

## How it is wired

- **A booked shipment reaches the plant through a link.** The hub's `shipment_booked` event calls the plant's
  `add_shipment` (`docs/protocol.md`, "Links between parts"). Agents don't get that tool.
- **The alarm needs no link.** It is on the plant's own event log, which agents read with `recent_events` and
  `wait_for_event`, and arenad records it as a world event.
- **Approvals bind through gates** (`docs/protocol.md`, "Gates on tools"), declared in `src/world.rs`. `book_freight`,
  `notify_customer`, `dispatch_technician` and `award_order` (whose `po` is the change's id) run only when the change
  they name is approved, is of the right action, and has the same fields as the call. Otherwise they answer with an
  error and the part is not called. The integrations stay as they were: neither knows the other.
- **Scoring is by named checks over the end state** (`src/checks.rs`), with no model judge and no gold world, since
  a change's id is made up as a run goes:
  `alarm_tripped`, `diagnosis_outer_race`, `qualified_technician_dispatched`, `correct_part_ordered_and_approved`,
  `purchase_order_in_erp`, `freight_booked_on_approval`, `shipment_reached_plant`, `repair_scheduled`,
  `customers_notified`. A branch passes when all pass. They are in the check's `checks`, and the ones that failed in
  its `differing_rows`.

## The team

`src/team.rs` holds the team it suggests: Maintenance, Procurement, Production planning, Finance, then the Incident
lead. The factory example's team is larger and works at the same time. Arena's teams are a sequence in which only
the last agent writes, so the first four read and hand off, and the lead does every write.

That has a cost: **the lead proposes and decides its own changes**, standing in for the plant manager, since a
change needs a decider and there is no other writer. So a gate proves a change was made, approved and followed
to the letter, not that someone else approved it. The limits ($5,000 for a purchase order, approved vendors, $800
for freight) are held by the checks, not by the approvals part, which has no rules here.

The repair itself isn't waited for: a truck takes hours of plant time. The task ends with the repair scheduled.

## The plant's backend

`arena-factory --backend standin|isaac` (default `standin`) picks the plant's integration; it is a program argument
because `describe` has no config. To use Isaac Sim, put a copy of this manifest under an `ARENA_PACKAGES` root with
`--backend isaac` in its command. **The `isaac` backend has never run in this world, or on Isaac Sim at all.**

## Tests

- `cargo test -p arena-factory` needs nothing: every named check against end states made in the test, the seed
  records, the gates' shape, and what it answers without the example.
- `(cd core && cargo test -p arenad --test factory -- --ignored live_ --nocapture)` runs the whole world with a
  scripted team and no model: the right path passes every check, and a wrong one (the wrong diagnosis and part, a
  booking with no change behind it) fails. It starts an ERPNext stack of its own and removes it after.

**Verified:** that live test passed on Linux, in about 190 s, against `frappe/erpnext:v16.53.0`, a local release build
of Sculptured and the stand-in plant, with the clock at 20 times real time. It covers one run of each path. **Not
run:** real agents or models, the `isaac` backend, a rerun or fork of this world, and `arena-part-check` on
`isaac-sim-standin`. Each new run makes arenad check that the base save exists, which on ERPNext rebuilds a scratch
site.
