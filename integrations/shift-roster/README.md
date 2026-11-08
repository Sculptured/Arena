# shift-roster

**A simulation.** Puts a shift roster in the world: the stations of a shift (press lines, robot cells), the one
certification each needs, and the operators with their certifications, on shift at a station, floating, or on call.
It stands in for time-and-attendance software such as UKG Workforce Central or SAP SuccessFactors; `describe.system`
is `{kind: "workforce", product: "Simulated shift roster", simulated: true}`. Restore: exact from a save (one
`state.json`); approximate: no. `uses_clock`: false.

Why a simulated part and not ERPNext: ERPNext's core has the Employee doctype but no skills, certifications or shift
assignments (those are in the separate HRMS app), and a call-out is a scheduled disruption, which only a simulated part
can hold (docs/protocol.md, "Scheduled disruptions").

## Config

Given by the factory benchmark (`benchmarks/factory/data/shift.json`, `roster`).

| key | what |
|---|---|
| `stations` | `[{id, name, kind, needs}]`, `needs` the certification an operator must have |
| `operators` | `[{id, name, certifications, station, on_shift?, on_call?}]` (`on_shift` defaults to true) |
| `shift` | shown as it is |
| `overtime` | `{rate_per_hour, hours}`: what calling one operator in costs |
| `scenarios` | optional scenarios by name, each with `disruptions` |

## Tools

| tool | effect | what |
|---|---|---|
| `shift_roster {}` | read | each station with its operator and whether it is staffed (and why not); each operator; overtime so far |
| `assign_operator {operator, station}` | write | puts an on-shift operator (id or name) at a station; whoever was there floats. An operator not certified for it is placed with a warning, and the station is not staffed |
| `call_in_operator {operator}` | write | an on-call operator joins the shift, on overtime |
| `roster_scenario {name, disruptions?}` | write | sets the scenario and schedules disruptions; a task's setup calls it, so agents don't get it |

Keys: `scenario/current`, `station/<id>` (`staffed`: its operator is on shift and certified for it), `operator/<id>`,
`overtime/shift`, `log/actions`, `_disruption/<id>` (internal). The factory benchmark gates the plant's `start_cell`
on `station/<cell>` being staffed.

**Scheduled disruption**: `callout {operators}`: the operators (ids or names) call out: they are off shift, and the
stations they staffed are not staffed. `roster_scenario` counts for none.

## Tests

`uv run --with pytest pytest -q`: the stations at the start, a call-out covered by a floater and a call-in, a station
taken over, save and restore, and `arena-part-check`. Verified on Linux, 2026-11-07: all 5 pass, the conformance check
included.
