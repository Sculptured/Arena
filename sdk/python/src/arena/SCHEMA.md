# Arena dataset (`arena-dataset/1`)

A folder exported by the Arena SDK (`Branch.export`, `Suite.export`, `arena.export.export_branches`). Every
line of a `.jsonl` file is one JSON object. Times are ISO 8601 in UTC. Fields a core did not record are `null`
or absent, never guessed.

## `branches.jsonl`: one line per branch

| Field | Meaning |
|---|---|
| `format` | `arena-dataset/1` |
| `run_id`, `branch_id` | ids; a run's first branch has `parent_id: null` |
| `parent_id`, `start_seq` | for a fork: the branch it was forked from and the step it starts at |
| `task_id`, `benchmark` | the benchmark's task the run works on |
| `seed`, `base_seed` | the seed the run's parts started with, and the seed of the shared base world every run starts from |
| `runner` | `chain`, `concurrent` or `external` (who drove the agents) |
| `scenario` | the expanded disruptions added to the task (`{disruptions: [{id, label, part, params, disruption}]}`), or null |
| `world` | the parts of the world: `[{part, integration, config?}]` |
| `team` | `{id, name, agents: [{id, name, kind, model, prompt, provider?, read_only}]}` as the branch ran |
| `status`, `error` | `passed`, `failed`, `infra_error`, `interrupted` (arenad restarted mid-run), `stopped` (newer cores), or `running` |
| `created_at`, `ended_at` | wall-clock start and end |
| `restore_mode`, `restored` | for a fork: `exact` or `approximate`, and how each part was restored (`save`, `replay`, `approximate`) |
| `fork_note` | what changed in the fork |
| `passed` | whether the branch passed (null until scored) |
| `checks` | `[{name, passed, detail, safety}]`: the benchmark's named checks; for a benchmark scored against its gold world, one `matches_gold` check |
| `score` | share of goal checks passed (safety checks left out) |
| `safety_violations` | names of failed safety checks |
| `differing_rows` | keys (`<kind>/<id>`, prefixed `part:` in a world of several parts) that differ from the task's gold world, or the failed named checks |
| `metrics` | `{measured_s, estimated_s, overhead_s, overhead_ms: {kind: ms}, tokens, cost: {usd, label, source, as_of}, agents: [{agent, kind, model, provider, tokens, prompt_tokens, completion_tokens, cost, model_calls, peak_prompt_tokens, compactions, overhead_s}]}`; `estimated_s` is the time without Arena's own overhead |
| `events` | number of events of the branch in `events.jsonl` |

## `events.jsonl`: one line per recorded event

| Field | Meaning |
|---|---|
| `run_id`, `branch_id`, `task_id` | where the event belongs |
| `seq` | the event's step number in its branch, from 1 |
| `at` | when it was recorded |
| `agent` | the agent it belongs to, or null for the world and arena |
| `kind` | see below |
| `inherited` | true for a fork's events before its fork point (copied from its parent; analyse a fork from `start_seq`) |
| `payload` | the kind's fields, below. Every payload may carry `overhead_ms` (`{restore, save, record, gateway, mcp, gate, link, wait, score, pause?}`): Arena's own time since the previous event |

Payloads by `kind`:

- `run_start`: `{task_id, team, benchmark, world, seed, base_seed, fingerprints, saves, runner, handoff, systems, tools, scenario?}`.
  `systems` maps each part that says so to `{kind, product, stands_in_for?, simulated}`; `tools` maps each tool
  to `{effect: "read"|"write", part?}`.
- `agent_start`: `{name, kind, model, provider?, external?}`.
- `model_call`: `{request, response, usage: {prompt_tokens, completion_tokens, total_tokens, ...}, cost_usd, cost_source, provider?, provider_response?}`
  in OpenAI chat-completions form whatever the provider. A call your client reported (`reported_by: "client"`)
  has `request` and `response` null; a compaction has empty `usage` and `compaction`.
- `tool_call`: `{name, args, result, error, reads, writes, internal?, part?, saves?, clock_ms?, paused_ms?, duration_ms?, tool_call_id}`.
  `error` is set for a refused or failed call; `reads`/`writes` are the keys the call read and changed;
  `saves` the parts saved after it; `duration_ms` how long it ran in the world.
- `handoff`: `{note, to?}`: a note left for the agents that start after it (or for `to`).
- `message`: `{to, text}`: a message to one agent, or to all (`to: "*"`).
- `agent_end`: `{ok, error, tokens, prompt_tokens?, completion_tokens?, cost_usd, cost_source, seconds}`.
- `world_event`: `{part, kind, text, data, clock_ms}`: something a part reported as its clock advanced
  (a disruption, a delivery, an alarm).
- `world_view`: `{seq_of, view}`: the benchmark's view of the world (the digital twin) after step `seq_of`.
- `check`: `{passed, gold_db, differing_rows, checks?, final_saves, unchecked}`: how the branch was scored.
- `pause`, `resume` (newer cores): a pause of the branch; `resume` carries `paused_ms`.

New kinds and fields may be added; readers should ignore what they don't know.

## What varies between runs

Every run of a task starts from the same seeded base world and is scored by the same checks. What varies is
the agents: models are not deterministic, so repeat a run (a suite with `repeats`) to measure a rate rather
than reading one run as the answer. Set `seed` on a run to fix the seed parts draw from (simulators, clocked
parts); it is recorded either way. A fork restored `approximate` is labelled in `restore_mode`; compare such
branches with that in mind.
