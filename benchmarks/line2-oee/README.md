# Line 2 OEE

An optimisation benchmark: an engineer's goal, a plant's simulation model and a budget of simulations.
Can an optimiser agent (or a team) find a configuration that reaches the goal, within the plant's
limits, and back it with evidence that holds up when re-simulated? And which optimiser design does it
best, with how many simulations, at what cost? Its world is one part, `line-sim`
(`integrations/line-sim`), a discrete-event model of the plant; Python, on the kit's `Benchmark`. No
Docker stack.

## The plant and the task

Ridgeline Pumps (fictional, `data/plant.json`): two lines that share two maintenance technicians.
Line 1 makes P-200 pumps (4 stations, committed to 2,650 good pumps a week). Line 2 makes V-50 and V-80
valves (machining, assembly, test, pack; campaigns of each variant with a changeover between them;
customers promised a 4-hour lead time). 13 people per shift. Line 2 runs at about 77% OEE (77.2% verified
on 20 replications).

The task `line2-oee` (the plant manager's ticket): get Line 2 to 85% OEE with no extra headcount, within
the USD 15,000 capex budget, without breaking Line 1's commitment or Line 2's lead-time promise, using at
most 30 simulations; recommend one simulated configuration with its evidence.

Decisions: Line 2's three buffer sizes, batch size, speed (85–115%), PM hours per week and crew per
station; Line 1's crew per station; technicians per shift. The hard limits (bounds, headcount, capex,
safety minimums: an operator at every station, a technician per shift, at least 2 h of PM per line) are
enforced by the part before anything is simulated; see `integrations/line-sim/README.md`.

## Checks (scored by named checks, never a model)

The part re-simulates the recommendation on 20 replications the study never used; the checks read that.

| Check | Kind | Passes when |
|---|---|---|
| `recommendation_submitted` | goal | a simulated config was recommended |
| `goal_reached` | goal | its verified Line 2 OEE is at least 85%. Its `measure` is the verified OEE: the run's score |
| `evidence_holds` | goal | the verified OEE is no more than 1.5 points below the simulation offered as evidence |
| `within_budget` | goal | at most 30 simulations were used. Its `measure` is the number used |
| `constraints_respected` | safety | the recommended config breaks no hard limit and, verified, keeps Line 1's commitment and Line 2's lead time |

## Three optimiser designs

All three are Claude Code agents with the same tools; the benchmark offers the first as the default team
and the others one click away.

- **Single optimiser** (one agent, `chain`): "an industrial engineer improving a plant with its
  simulation model".
- **Planner and analyst** (`dispatch`): the planner decides what to try and submits the recommendation;
  the analyst runs the simulations it is given and reports the results and the bottleneck.
- **Optimiser with a search strategy** (one agent, `chain`): the single optimiser plus a strategy:
  baseline, bottleneck from station states, one lever at a time then combine, look for idle capacity to
  move within the headcount, screen with 3–5 replications and confirm with 10.

## Reference

`scripts/reference.py` runs a plain random search offline, with the same budget and the same part:
today's config first, then 29 configs drawn uniformly within the bounds (skipping any a hard limit would
deny), 5 replications each, then the best feasible one verified on 20 new replications. Over 10 searches
(`data/reference.json`, made 2026-11-08) the verified OEE was 77.2% median, 80.3% best; none reached 85%:
random configs rarely beat today's. It is a number beside the score (`goal_reached`'s
`measure.reference`), never a run.

## Run it

```
ARENA_BENCHMARK=line2-oee cargo run -p arenad      # from core/; the parts run with uv
(cd integrations/line-sim && uv run --with pytest pytest -q)
(cd benchmarks/line2-oee && uv run --with pytest pytest -q)
(cd benchmarks/line2-oee && uv run python scripts/reference.py 10)   # remakes data/reference.json
```
