# ridgeline-hub

The simulated outside companies of the Ridgeline factory demo (suppliers, carriers, the insurer and
customers) as an `arena-part/1` integration, running on the arena clock. It wraps the factory hub's
`ridgeline_hub.market.Market` and changes only where time comes from: `now()` returns arena seconds, not
`time.monotonic()`. Replies therefore arrive at the same arena times in every run and replay.

## Config

| key | default |
|---|---|
| `hub_path` | `Sculptured-Examples/factory/hub`, next to the arena checkout |
| `data_path` | `<hub_path>/../data/ridgeline.json` (its `suppliers` list) |
| `plant_start` | `2026-03-10T06:00:00` (the plant clock at arena time 0) |

Only `ridgeline_hub.market` is imported (through `hub_path` on `sys.path`). A missing path or module makes
`start` fail with `-32602` naming the path. The factory folder is read only and never modified.

## Behaviour

- `uses_clock: true`. The market ticks every 500 ms of arena time. `call_tool` first advances to its
  `clock_ms`, then runs the tool, so a replay at the same clock times gives the same state. Events from
  the ticks (a quote arrived, a customer replied) stay queued and the next `advance` returns them.
- Tools are those of the factory's `ridgeline_mcp/market.py`: `request_quotes`, `get_quotes`,
  `award_order`, `freight_quotes`, `get_freight_quotes`, `book_freight`, `submit_claim`, `answer_claim`,
  `notify_customer`, `customer_replies`. They return at once. `wait_s` is accepted and ignored: blocking
  inside `call_tool` would stall the world, so agents poll while the arena clock runs.
- `proposal_id` (on `book_freight` and `notify_customer`) is recorded on the stored object and not
  verified. Approval gating is the `sculptured` integration's job.
- Unknown ids and bad arguments are `-32602` errors naming the id.
- Keys for `inspect`, `diff` and tool reads/writes: `rfq/<id>`, `freight/<id>`, `claim/<id>`,
  `notice/<id>`, `shipment/<id>`.
- A booked freight quote creates `shipment/SHP-001`, ... with its arrival on the plant clock. The shipment
  is kept in this part only. It is not forwarded to Isaac Sim: protocol v1 has no cross-part handoff.
- Restore is exact from a save: one `state.json` in the save folder with all market state, the shipments,
  the queued events and the stepper position. The market has no randomness, so `seed` is accepted and
  unused; the replay contract holds for any seed (`describe` says so in its fingerprint).
- `usage` is 0 (everything is under `ARENA_SAVES`). `gc` removes save folders not kept.

## Tests

```
uv run --with pytest pytest -q
```

The conformance test runs `arena-part-check` and is skipped when the binary is not built
(`cargo build -p parts --bin arena-part-check`).
