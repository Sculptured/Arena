# utility

**A simulation.** Puts an electric utility in the world: the site's main meter, its tariff and the utility's
demand-response events, where the site is asked to hold its load at or below a target during a window of peak
prices. The meter is fed from the plant's load: a benchmark links the plant's `power` events to `meter_load`. It
stands in for a utility's demand-response program and a revenue meter; `describe.system` is `{kind: "utility",
product: "Simulated electric utility and site meter", simulated: true}`. Restore: exact from a save (one
`state.json`); approximate: no. `uses_clock`: true.

A Python package on the kit's `arena_part.jsonstate`, with the clock added: `advance` (and every `call_tool`, first)
integrates the load up to the Shakedown clock. The load is kept in whole watts and the energy in whole watt-milliseconds,
cut at every window's start and end, so the state after an advance doesn't depend on how the advances were split.

## Config

Given by the factory benchmark (`benchmarks/factory/data/shift.json`, `utility`). Every key has a default.

| key | what |
|---|---|
| `meter`, `site`, `utility` | names shown in the meter's document |
| `plant_start` | the plant clock at Shakedown time 0 (default `2026-03-10T06:00:00`) |
| `initial_kw` | the load until the plant's first reading (default 1500) |
| `tariff` | `{standard_per_kwh, peak_per_kwh}` in USD; inside a demand-response window the peak price applies |
| `scenarios` | optional scenarios by name, each with `disruptions` |

A negative `initial_kw`, a tariff without both prices or a bad `plant_start` is `-32602`.

## Tools

| tool | effect | what |
|---|---|---|
| `utility_meter {}` | read | the load now, the peak so far, the energy and its cost this shift, the tariff period and price |
| `demand_response_events {}` | read | each event: target, window (plant time), status (`notified`, `active`, `ended`), the site's response, the minutes inside the window so far, the average and highest kW there and the minutes over the target |
| `respond_to_event {event, committed_kw, plan}` | write | the site's answer to an event (refused once it has ended) |
| `meter_load {kw, cause?}` | write | a reading of the plant's load; a link calls it, so agents don't get it |
| `utility_scenario {name, disruptions?}` | write | sets the scenario and schedules disruptions; a task's setup calls it, so agents don't get it |

Keys: `scenario/current`, `meter/site`, `dr/<event>`, `_disruption/<id>` (internal). A window's start and end are
world events, `demand_response_active` and `demand_response_ended` (from `advance`; one found during a call waits for
the next `advance`).

**Scheduled disruption** (`arena_part.disruptions`, docs/protocol.md "Scheduled disruptions"): `demand_response {event,
target_kw, lead_min, minutes}`. When it fires its notice is there (`notified`); the window opens `lead_min` minutes of
plant time later and lasts `minutes`. `meter_load` and `utility_scenario` count for no disruption.

## Tests

`uv run --with pytest pytest -q`: the meter on the clock, split advances, a window measured, a notice after its count,
responses and refusals, save and restore, and `arena-part-check` (skipped without the built checker; `ARENA_BIN_DIR` names
where it is). Verified on Linux, 2026-11-07: all 9 pass, the conformance check included (replay skipped since the part
restores from saves).
