# ridgeline-hub

Puts the Ridgeline factory's outside companies in the world (suppliers, carriers, the insurer, customers), as the
factory hub's `Market` running on the arena clock. Restore: exact from a save (one `state.json`); approximate: no.
`uses_clock`: true. Verified offline only: its pytest suite (`uv run --with pytest pytest -q`) imports the real
`market.py` from the factory demo folder and runs the `arena-part-check` conformance check. No service or
container is involved. No arenad test or run has used it.

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
`start` fail with `-32602` naming the path. The factory folder is read only and never modified. `hub_path` is process-global (`sys.path` and
`sys.modules`), so one integration process serves one hub. `describe` fingerprints the sha256 of `market.py`
and `ridgeline.json` at the default paths; `start` refuses (`-32003`) configured files that differ.

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
- Unknown ids, an unknown supplier or quote, a `book_freight` carrier or amount that doesn't match the quote,
  and bad tool arguments come back as a tool-level `error` result and change nothing. An unknown tool name
  is `-32602`.
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
