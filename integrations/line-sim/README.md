# line-sim

A simulation study of a plant, as one part: an agent proposes configurations, simulates them on a
discrete-event model, reads the KPIs and recommends one. The part enforces the study's hard limits and
its simulation budget, records every simulation, and verifies the recommendation by re-simulating it on
replications the study never saw. Python, on the kit's `JsonStatePart` (`kits/python`). The plant it
simulates comes in its config; `benchmarks/line2-oee` gives it a two-line valve and pump plant.

## The model (`line_sim/des.py`)

A compact discrete-event simulation of serial flow lines that share one maintenance crew, written for
this part (no SimPy). Per line: stations with finite buffers between them (blocking after service);
processing times that depend on the crew at the station, the line speed and the variant, with lognormal
variation; machines that fail on busy time (MTBF, shorter at higher speed) and wait for a free
technician to repair them (MTTR); a changeover whenever the variant changes (shorter with more
operators); campaigns of each variant sized by the batch size and the mix; planned maintenance (PM) at
the start of each shift, which stops the line until a technician has done it and lengthens MTBF; test
scrap that rises above 100% speed. A week is 10 eight-hour shifts after a 4-hour warm-up.

**Deterministic, with common random numbers.** Every draw comes from its own stream, seeded by
`(seed, replication, line, station, purpose)` (SHA-512 of the name, stable across processes). So
replication r of two configs meets the same failures, repair times and variation wherever they agree,
and the difference between two configs is mostly the configs', not noise. The seed is the plant's
(`seed` in the config), not the run's, so every run of a benchmark simulates the same weeks.

## KPIs (`line_sim/kpis.py`)

Per replication, then mean and 95% confidence half-width (Student's t) over the replications:
`line2.oee`, `line2.quality`, `line2.good_per_week`, `line2.wip`, `line2.lead_time_h`, `line1.oee`,
`line1.good_per_week`, `cost_per_week_usd`. Each station's share of time working, in changeover, starved,
blocked, down and in PM, and its failures per week. OEE is good units × the line's ideal cycle time over
all staffed time, so PM, changeovers, failures, waiting for a technician, blocking and starving all count
as losses. Lead time is the mean flow time plus half the campaign cycle (an order waits on average half a
cycle for its variant's next campaign).

## Tools

| Tool | Effect | What it does |
|---|---|---|
| `describe_model` | read | Lines and stations, every decision (today's value, bounds, unit), the hard limits, the limits judged after a simulation, the KPIs, the budget |
| `run_simulation(config, replications?)` | write | Simulates `config` (decision name → value; left out: today's value) on replications 0…n−1 (default 5, at most 10). Answers the KPIs, station states, the after-simulation limits and `objective` (`{name, label, value, ci95, unit, better}`) |
| `get_history` | read | Every simulation: changes, objective, feasibility, key KPIs; the best feasible one; the budget left |
| `submit_recommendation(simulation, summary)` | write | Recommends one simulated config. Final. The part re-checks the hard limits and re-simulates it on 20 replications from 1000 on, which no `run_simulation` uses |

Each simulation is a step with its config and KPIs, a `simulation` world event ("Simulation 7: Line 2 OEE
81.3% ± 0.6 (5 replications)") and a doc `simulation/<n>`. The recommendation is a `recommendation` event
and the doc `recommendation/final`; the budget is `study/budget`.

## Limits

Checked before anything is simulated, deterministically, from the plant's `decisions` and `safety`:
bounds of each decision, headcount per shift (every operator on both lines plus the technicians), the
capex budget (each buffer slot above today's), and safety minimums (an operator at every station, a
technician per shift, PM hours per line). A config that breaks any of them is refused with a structured
denial naming every rule it broke (`{"denied": true, "code": "CONFIG_REJECTED", "violations": [{rule,
param, value, limit, message}]}`), recorded in `_audit/denials`, reported as a `denial` event, and costs
no simulation. A simulation past the budget gets `SIMULATION_BUDGET_EXHAUSTED`. A malformed call (an
unknown decision, a wrong type) is an ordinary refusal, not a denial. Limits that only a simulation can
judge (Line 1's weekly commitment, Line 2's lead-time promise) are reported with each result (`constraints`,
`feasible`), never denied.

## Backends (`line_sim/backend.py`)

The part talks to a simulator only through `SimulationBackend`: `replicate(plant, seed, reps)` answers
one raw result per replication number, and `fingerprint()` says what decides the results. Validation,
the budget, KPIs, intervals and the record are the part's, whatever the backend. The config's `backend`
picks it (default `des`, the model above, in process). Any other name is refused at `start`.

### A remote simulator (not built)

A commercial discrete-event simulator (Rockwell Arena Simulation or Emulate3D) runs on a licensed Windows
machine. It is meant to plug in as a worker over HTTP, behind a `SimulationBackend` named for it. The
adapter is not written; this is the contract it and the worker must meet:

- `POST /simulate` with `{"model": "<model id and version>", "plant": {...}, "seed": 2026, "replications": [0, 1, 2]}`:
  `plant` is the plant with the config applied, in this part's shape (`lines`, `stations`, `buffers`,
  `crew`, `batch_size`, `speed_pct`, `pm_hours`, `maintenance.techs`, `week`). The worker maps it onto its
  model's controls.
- Answer `{"replications": [<raw result>, ...]}` in the order asked, each in the shape `replicate`
  documents (`measured_s`, and per line `good`, `scrap`, `wip_mean`, `flow_mean_s`, `campaign_cycle_s`,
  `pm_s`, and per station `failures` and `share` of time in `working`, `changeover`, `starved`,
  `blocked`, `down`, `pm`).
- Replication r with the same seed gives the same result (the simulator's own random-number streams
  keyed by seed and replication, as common random numbers need).
- `GET /model` answers the model id, its version and a hash of the model file, for `fingerprint()`.
- A config the model can't take is a 422 with a reason; the part reports it as an ordinary refusal.
- Timeouts: one replication of a week should answer in seconds; the adapter waits at most 60 s per call.

## Run the tests

```
(cd integrations/line-sim && uv run --with pytest pytest -q)
```
