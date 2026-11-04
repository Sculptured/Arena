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

`src/team.rs` holds the team it suggests. As in the factory example, each role does its own work with its own
tools, and a change is decided by another role than the one that proposed it:

| Role | Does | Proposes | Decides |
|---|---|---|---|
| Maintenance | Diagnoses the failure from the evidence, names the part and a qualified technician | `maintenance_order` | |
| Procurement | Requests and compares quotes, and freight quotes from the vendor | `purchase_order`, `freight_booking` | |
| Finance | Awards the approved order and records it in the ERP | | `purchase_order`, `freight_booking` |
| Production planning | Books the approved freight, schedules the repair, finds the affected orders | `customer_notice` | |
| Incident lead | Dispatches the technician and notifies the customers on approved changes | | `maintenance_order`, `customer_notice` |

Arena runs a team in order, each agent handing off to the next, so a change can only be acted on by the role that
decided it or a later one. That is where this team differs from the example, where the proposer acts once its change
is approved: here Finance awards the order Procurement proposed, Planning books the freight, and the lead dispatches
the technician Maintenance chose. The example's EHS, Customer notices and Insurance roles are left out, since no
check covers their work. No agent is read-only.

The approvals part doesn't know which agent decided a change (arena's event log does), and it has no rules here, so
the limits ($5,000 for a purchase order, approved vendors, $800 for freight) are held by the checks, and the team
keeping proposer and decider apart is up to its prompts.

The repair itself isn't waited for: a truck takes hours of plant time. The task ends with the repair scheduled.

## The plant's backend

`arena-factory --backend standin|isaac` (default `standin`) picks the plant's integration; it is a program argument
because `describe` has no config. To use Isaac Sim, put a copy of this manifest under an `ARENA_PACKAGES` root with
`--backend isaac` in its command, or set `ARENA_RUN_FACTORY` to that command line (`docs/protocol.md`).
`--plant-config '<JSON object>'` sets the plant part's config, such as `isaac-sim`'s `record` (an MP4 of the run).
With `ARENA_RUN_ISAAC_SIM` the plant can run on a GPU machine over ssh (`integrations/isaac-sim/README.md`).

## Tests

- `cargo test -p arena-factory` needs nothing: every named check against end states made in the test, the seed
  records, the gates' shape, and what it answers without the example.
- `(cd core && cargo test -p arenad --test factory -- --ignored live_ --nocapture)` runs the whole world with a
  scripted team and no model, each agent doing its own part as in the table above: the right path passes every check, and a wrong one (the wrong diagnosis and part, a
  booking with no change behind it) fails. It starts an ERPNext stack of its own and removes it after.

**Verified:** that live test passed on Linux, in about 190 s, against `frappe/erpnext:v16.53.0`, a local release build
of Sculptured and the stand-in plant, with the clock at 20 times real time. It covers one run of each path. That was
with the earlier script, in which the lead made every change; the script with the work divided between the roles
compiles but has not been run yet. **Not
run:** real agents or models, a rerun or fork of this world, and `arena-part-check` on
`isaac-sim-standin`. Each new run makes arenad check that the base save exists, which on ERPNext rebuilds a scratch
site.

**Verified with Isaac Sim (2026-11-04):**
- What ran: `live_factory_gold_path_report`, which runs only the gold path and prints its checks and metrics. It
  used `ARENA_RUN_FACTORY="… arena-factory --backend isaac --plant-config '{"record": …}'"` and
  `ARENA_CLOCK_SPEED=0.1`.
- Where: the plant ran on Isaac Sim 6.2.0 on a rented RTX 3090 (RunPod), over ssh. The rest of the world ran
  locally.
- Result: it passed all nine checks. It measured 1311.0 s, with 156.9 s of overhead, so 1154.1 s estimated, and
  recorded the run to MP4. Details are in `integrations/isaac-sim/README.md`.
- The wrong path has not been run on Isaac Sim.
