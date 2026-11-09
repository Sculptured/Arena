# Agent-driven configuration optimiser

Approved in chat 2026-11-08; built on the branch `optimizer`.

## Why

An engineer states a goal ("Line 2 from 77% to 85% OEE with no extra headcount"). An optimiser agent, or
a team, proposes plant configurations, simulates each, reads the KPIs, iterates, and recommends one with
evidence. The product records every candidate and simulation, and an experiment compares optimiser
designs on the same plant and goal: best verified result, simulations used, model cost, time. Later the
simulation runs on a commercial simulator (Rockwell Arena Simulation or Emulate3D, licensed, Windows);
today it runs on an open-source discrete-event model behind the same interface.

## What is built

- **`integrations/line-sim`**, a part: a compact discrete-event model of serial lines sharing a maintenance
  crew (stations, finite buffers, MTBF/MTTR failures waiting for technicians, changeovers, campaigns,
  planned maintenance per shift, crews, line speed, test scrap), deterministic per (seed, replication)
  with a random stream per station and purpose (common random numbers). Tools: `describe_model`,
  `run_simulation(config, replications?)` (KPIs with 95% intervals, station states, after-simulation
  limits, `objective`), `get_history`, `submit_recommendation(simulation, summary)`.
- **Hard limits enforced by the part**, before anything is simulated: bounds, headcount per shift, the
  capex budget, safety minimums; a structured denial naming every broken rule, recorded and reported as a
  `denial` event; costs no simulation. The simulation budget is enforced the same way.
- **Verification**: the recommendation is re-simulated on 20 replications (1000–1019) no study call uses;
  the hard limits are re-checked.
- **`SimulationBackend`** (`line_sim/backend.py`): `replicate(plant, seed, reps)` and `fingerprint()`;
  `des` is the only backend. The contract for a remote simulator (a Windows worker over HTTP) is in
  `integrations/line-sim/README.md`; it is not built, and any other backend is refused.
- **`benchmarks/line2-oee`**: the plant (`data/plant.json`), the task, three optimiser designs (single
  agent; planner and analyst on the `dispatch` runner; single agent with a search strategy), five named
  checks (recommendation submitted, goal reached, evidence holds, within budget; constraints respected as
  the safety check), a twin view, and `scripts/reference.py`.
- **Protocol**: a named check may carry a `measure` (`{label, value, unit?, better?, target?,
  reference?}`); a tool result with `objective` is a candidate (docs/protocol.md). The core change is that
  one optional field on `NamedCheck` (`kits/rust/src/proto.rs`); arenad stores and reports it unchanged.
- **UI**: on a run, "Best so far" (each candidate's objective by simulation number with its interval,
  the best feasible step line, the target, the verified value; a dot opens its step). On an experiment,
  each measure per setup (the best run, the mean, every run as a dot on a scale all setups share, the
  target and the reference as ticks).

## Decisions

- **Own DES, not SimPy**: no dependency, about 0.1 s per replication-week in pure Python, and full control
  of the random streams for common random numbers.
- **The plant's seed, not the run's**: every run simulates the same weeks, so designs are compared on
  equal terms and a config's verified value is the same in every run.
- **OEE over all staffed time**: planned maintenance counts as a loss, so PM trades off against failures
  instead of being free.
- **Budget counts calls, not replications** (1–10 per call): the agent chooses between screening and
  confirming, and the `evidence_holds` check rewards confirming.
- **Outcome limits are reported, not denied**: Line 1's commitment and Line 2's lead time are only known
  after simulating; the recommendation is judged on its verified values (safety check).
- **The reference is a random search** from today's config with the same budget, run offline, shown as
  `measure.reference` and in the README. Never a run.
- **One recommendation, final**: no resubmitting after seeing the verification.
- **Teams use prompts for roles**: no per-agent tool restriction on this branch (another branch adds it).

## Calibration (2026-11-08, from the model, not from runs)

Today's config: 77.2% verified (20 replications). Moving Line 1's second packer to Line 2's machining
(shorter changeovers) gave 82.8% on 5 replications; that plus campaigns of 240, PM at the 2 h safety
minimum, 105% speed and a first buffer of 15 gave 88.4% on 10 replications and 88.9% verified. Random
search with 30 simulations: 77.2% median, 80.3% best over 10 searches; none reached 85%.

## Not built

The Rockwell / Emulate3D adapter and its Windows worker; per-agent tool restriction for the analyst;
several tasks or plants; disruptions (a scenario) on the study.
