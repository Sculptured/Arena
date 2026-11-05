# factory

A benchmark of a whole incident across five systems at a fictional stamping plant. A press's spindle bearing fails. A
team has to find out what failed from the evidence, source the right part with approval, get it on a truck, schedule the
repair, tell the customer of every open order on the press's line due within three days and record that in the CRM. Its
world has five parts:

| Part | Integration | What it holds |
|---|---|---|
| `erp` | `erpnext` | The company below: its items, MES, maintenance, stock and orders; purchase orders, to-dos |
| `hub` | `ridgeline-hub` | The simulated suppliers, carriers and customers |
| `plant` | `isaac-sim-standin`, or `isaac-sim` with `--backend isaac` | The plant, on the arena clock |
| `approvals` | `sculptured` | Changes proposed and decided |
| `crm` | `twenty-crm` | The same customers as accounts, with contacts and an opportunity per order |

## The company

**Ridgeline Components is fictional.** Every name, order and number is invented (`data/company.json` says so, and so does a
Note in the ERP); the plant is simulated and the systems are open-source stand-ins. `seed` writes it once into the base world
that every run starts from, dated from the plant's first day, 2026-03-10 (the `plant_start` of the hub and the plant, so the
ERP, the CRM and the plant keep one calendar).

**At the plant's scale** (`src/scale.rs`, version 0.3.0): besides the factory example's three customers and eight orders,
the company has 37 more customers (2 or 3 contacts each in the CRM), 50 more stamped parts with their BOMs and stock, and two
months of open orders on every line: 1,083 orders in all, each a sales order and a work order with its job card in the ERP
and an opportunity in the CRM. They are generated from a fixed seed, so every base is the same and the score knows which
orders the outage affects: the 16 orders on Line A due within three days (`AFFECTED_DAYS`), from 14 customers. No note
lists them; the team finds them in the ERP. Generated orders' ids look like customers' PO numbers (`PO-41006`).

| Where | What |
|---|---|
| ERP | A company with its chart of accounts, as the site's default; warehouses for two plants |
| ERP, MES | Press lines A, B, C and F as workstations; items, BOMs with a stamping operation; a submitted work order for each order, on its line, with its job cards |
| ERP, maintenance | Each press as an asset; its plan (`Asset Maintenance`) and its history as logs |
| ERP, purchasing | Suppliers; list prices, and what each supplier last charged, in `Standard Buying` |
| ERP, stock | Opening stock. The failing press's part, `SP-SPN-800`, has none, against a shelf minimum of 1 |
| ERP, sales | The customers, and a submitted sales order per order (`po_no` is the order's id, like `A-1` or `PO-41006`) |
| CRM | The customers as accounts (with domain, LinkedIn, revenue and address), their contacts, and an open opportunity per order, closing on its due date |

**Build it:** `ARENA_BENCHMARK=factory cargo run -p arenad -- base` from `core/`, after building the packages, with the Postgres
of `deploy/compose.yml` running, a current Sculptured on `PATH`, the images of the five parts present, and arenad stopped. It makes the base world unless it exists, and exits; a run makes it too if
it is missing. The dates stay as they were when it was made: `factory`'s version names the base, so bump the version to
build another. On a machine where the build and saves are elsewhere:
```bash
export CARGO_TARGET_DIR=/path/target ARENA_BIN_DIR=/path/target/debug ARENA_SAVES=/path/saves
cargo build -p arena-factory -p arena-erpnext -p arena-twenty-crm -p arena-sculptured
(cd core && ARENA_BENCHMARK=factory cargo run -p arenad -- base)
```

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
  `customers_notified` (every customer with an affected order), `affected_orders_noticed` (an approved notice sent for
  every affected order), `crm_accounts_updated` (a note on the account of every customer with affected orders) and
  `crm_close_dates_moved` (for each approved notice, its order's opportunity closes on the notice's new date). A branch passes
  when all pass. They are in the check's `checks`, and the ones that failed in
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
| Incident lead | Dispatches the technician, notifies the customers on approved changes and records that in the CRM | | `maintenance_order`, `customer_notice` |

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
  booking with no change behind it) fails. It starts an ERPNext stack and a Twenty server of its own and removes them after.
  It needs a current Sculptured (`ARENA_SCULPTURED_BINARY`, a release build next to the checkout, or `PATH`); one too old
  to have the approvals API fails the base save with "no API route". With `CARGO_TARGET_DIR` set, set `ARENA_BIN_DIR` to match.

**Verified:** that live test passed on Linux, in about 190 s, against `frappe/erpnext:v16.53.0`, a local release build
of Sculptured and the stand-in plant, with the clock at 20 times real time. It covers one run of each path. That was
with the earlier script, in which the lead made every change; the script with the work divided between the roles
compiles but has not been run yet. **Not
run:** real agents or models, a rerun or fork of this world, and `arena-part-check` on
`isaac-sim-standin`. Each new run makes arenad check that the base save exists, which on ERPNext rebuilds a scratch
site.

**Verified at the plant's scale (version 0.3.0, 2026-11-05):** `arenad base` built the base world on Linux in about 9
minutes (`frappe/erpnext:v16.53.0`, `twentycrm/twenty:v2.51.0`, a debug build of Sculptured); while it ran, the ERP held
1,083 sales orders, work orders and job cards. Sales orders are written five at a time and work orders one at a time
(they all update the coil's stock row); the CRM is written through Twenty's batch endpoint, since Twenty limits an API key
to 100 requests a minute (agents' CRM calls meet the same limit). `cargo test -p arena-factory` passes. **Not run at this
scale:** the live scripted test, which now finds the affected orders in the ERP and notifies all of them. The
verifications below are of version 0.2.0 (three customers, eight orders).

**Verified with the company and the CRM (2026-11-04):** `live_factory_right_path_passes_and_wrong_path_fails` passed on
Linux, in about 290 s, with the seed above, `frappe/erpnext:v16.53.0`, `twentycrm/twenty:v2.51.0`, a debug build of Sculptured,
the stand-in plant and the scripted team, including its new CRM step: the right path passed all eleven checks. Counted once in
the ERP after seeding, without a run: 2 companies (`Arena Co`, from the integration's base, stays), 4 workstations, 5 BOMs, 8
sales orders, 8 work orders, 8 job cards, 4 assets, 15 item prices, and no stock row for `SP-SPN-800`. `arenad base` took
170 s the first time, including the integrations' own bases, and 23 s when the base existed. **Not run:** this world on
Isaac Sim or with real agents; the Isaac Sim result below predates the CRM.

**Verified with Isaac Sim (2026-11-04):**
- What ran: `live_factory_gold_path_report`, which runs only the gold path and prints its checks and metrics. It
  used `ARENA_RUN_FACTORY="… arena-factory --backend isaac --plant-config '{"record": …}'"` and
  `ARENA_CLOCK_SPEED=0.1`.
- Where: the plant ran on Isaac Sim 6.2.0 on a rented RTX 3090 (RunPod), over ssh. The rest of the world ran
  locally.
- Result: it passed all nine checks. It measured 1311.0 s, with 156.9 s of overhead, so 1154.1 s estimated, and
  recorded the run to MP4. Details are in `integrations/isaac-sim/README.md`.
- The wrong path has not been run on Isaac Sim.
