# ridgeline-hub

Puts the Ridgeline factory's outside companies in the world (suppliers, carriers, the insurer, customers), as the
factory hub's `Market` running on the Shakedown clock. Restore: exact from a save (one `state.json`); approximate: no.
`uses_clock`: true. Verified offline only: its pytest suite (`uv run --with pytest pytest -q`) imports the real
`market.py` from the factory demo folder and runs the `arena-part-check` conformance check. No service or
container is involved. No arenad test or run has used it.

**The factory example this package wraps is not public yet.** Its default `hub_path` points into a private
`Sculptured-Examples` checkout next to the `Arena` checkout; without one the package can't start and every test in
`tests/` is skipped (pytest says why). Outside contributors can read and change the code, but can't run it.

The simulated outside companies of the Ridgeline factory demo (suppliers, carriers, the insurer and
customers) as an `arena-part/1` integration, running on the Shakedown clock. It wraps the factory hub's
`ridgeline_hub.market.Market` and changes only where time comes from: `now()` returns Shakedown seconds, not
`time.monotonic()`. Replies therefore arrive at the same Shakedown times in every run and replay.

## Config

| key | default |
|---|---|
| `hub_path` | `Sculptured-Examples/factory/hub`, next to the `Arena` checkout |
| `data_path` | `<hub_path>/../data/ridgeline.json` (its `suppliers` list) |
| `plant_start` | `2026-03-10T06:00:00` (the plant clock at Shakedown time 0) |
| `inbound` | none. Raw-material trucks on their way: `[{id, material, supplier, carrier, eta_hours, line?, line_stock_hours?, ...}]` (see below) |

Only `ridgeline_hub.market` is imported (through `hub_path` on `sys.path`). A missing path or module makes
`start` fail with `-32602` naming the path. The factory folder is read only and never modified. `hub_path` is process-global (`sys.path` and
`sys.modules`), so one integration process serves one hub. `describe` fingerprints the sha256 of `market.py`
and `ridgeline.json` at the default paths; `start` refuses (`-32003`) configured files that differ, before it
imports anything.

## Behaviour

- `uses_clock: true`. The market ticks every 500 ms of Shakedown time. `call_tool` first advances to its
  `clock_ms`, then runs the tool, so a replay at the same clock times gives the same state. Events from
  the ticks (a quote arrived, a customer replied) stay queued and the next `advance` returns them.
- Tools are those of the factory's `ridgeline_mcp/market.py`: `request_quotes`, `get_quotes`,
  `award_order`, `freight_quotes`, `get_freight_quotes`, `book_freight`, `submit_claim`, `answer_claim`,
  `notify_customer`, `customer_replies`. They return at once. `wait_s` is accepted and ignored: blocking
  inside `call_tool` would stall the world, so agents poll while the Shakedown clock runs.
- `proposal_id` (on `book_freight` and `notify_customer`) is recorded on the stored object and not
  verified here. A benchmark can gate these tools on the `sculptured` part (`docs/protocol.md`, "Gates on tools").
- Unknown ids, an unknown supplier or quote, a `book_freight` carrier or amount that doesn't match the quote,
  and bad tool arguments come back as a tool-level `error` result and change nothing. An unknown tool name
  is `-32602`.
- Keys for `inspect`, `diff` and tool reads/writes: `rfq/<id>`, `freight/<id>`, `claim/<id>`,
  `notice/<id>`, `shipment/<id>`.
- A booked freight quote creates `shipment/SHP-001`, ... with its arrival on the plant clock, and the next
  `advance` gives a `shipment_booked` event whose `data` is the shipment. A benchmark's link can hand it to the
  plant (`docs/protocol.md`, "Links between parts"); the part itself knows nothing of the plant.
- Restore is exact from a save: one `state.json` in the save folder with all market state, the shipments,
  the queued events and the stepper position. The market has no randomness, so `seed` is accepted and
  unused; the replay contract holds for any seed (`describe` says so in its fingerprint).
- `usage` is 0 (everything is under `ARENA_SAVES`). `gc` removes save folders not kept.

## Inbound deliveries and their delay

The Market has no raw-material trucks; Shakedown adds them beside it (`ridgeline_hub_part/inbound.py`), for the factory
benchmark's shift task. A world without `inbound` sees the hub as before, except for the two tools below.

| tool | effect | what |
|---|---|---|
| `inbound_deliveries {}` | read | each truck: material, supplier, carrier, line, `eta` (plant time: `plant_start` plus `eta_hours` plus any delay), `status` (`in transit` or `arrived`, from the hub's clock), `line_runs_out` (plant start plus `line_stock_hours`) and `line_short_hours` |
| `hub_scenario {name, disruptions?}` | write | sets the scenario (`base`) and schedules disruptions; a task's setup calls it, so agents don't get it |

- Keys: `inbound/<id>`, `scenario/inbound`, `_disruption/<id>` (internal).
- **Scheduled disruption** (`arena_part.disruptions`, docs/protocol.md "Scheduled disruptions"): `delivery_delay
  {delivery, extra_hours}`: the truck arrives `extra_hours` later. It fires before the agent's call (of any hub tool,
  or of `when.tool`) that crosses its count, and stays when that call is refused; `hub_scenario` counts for nothing.
  The schedule is saved with the rest of the state.
- Nothing is linked to the plant: the plant doesn't model material, so a late coil doesn't stop a line by itself.
  `line_runs_out` says when it would.

## Tests

```
uv run --with pytest pytest -q
```

The conformance test runs `arena-part-check` and is skipped when the binary is not built
(`cargo build -p parts --bin arena-part-check`; `ARENA_BIN_DIR` names where it is). Verified on Linux, 2026-11-07:
all 23 pass (`tests/test_inbound.py` included), the conformance check included.
