"""Export a branch (with its forks) or a suite as a dataset, then read it back for a first look.

    uv run python examples/export_dataset.py <branch_id|suite_id> [out_dir]

No model calls. The folder has events.jsonl, branches.jsonl, SCHEMA.md (and suite.json). With pandas:
``pd.read_json("events.jsonl", lines=True)``.
"""

import collections
import os
import sys

from arena import NotFound
from arena.export import export_branches, read_jsonl
from common import connect

if len(sys.argv) < 2:
    sys.exit(__doc__)
target = sys.argv[1]
out = sys.argv[2] if len(sys.argv) > 2 else f"arena-dataset-{target[:8]}"

with connect() as arena:
    try:
        suite = arena.get_suite(target)
        suite.export(out)
    except NotFound:
        branch = arena.branch(target)
        # The run's other branches (its forks) go with it.
        run = next(r for r in arena.runs() if r.id == branch.run_id)
        export_branches(arena, [b["id"] for b in run.branches], out)

branches = read_jsonl(os.path.join(out, "branches.jsonl"))
events = read_jsonl(os.path.join(out, "events.jsonl"))
print(f"{out}: {len(branches)} branches, {len(events)} events")
for b in branches:
    print(f"  {b['branch_id'][:8]} task {b['task_id']} seed {b['seed']} {b['status']:<8} score {b['score']} "
          f"{'fork of ' + b['parent_id'][:8] + ' at ' + str(b['start_seq']) if b['parent_id'] else 'original'}")
own = [e for e in events if not e["inherited"]]
print("events by kind:", dict(collections.Counter(e["kind"] for e in own)))
tools = collections.Counter(e["payload"]["name"] for e in own if e["kind"] == "tool_call")
print("tool calls:", dict(tools.most_common(10)))
errors = sum(1 for e in own if e["kind"] == "tool_call" and e["payload"].get("error"))
print(f"tool calls refused or failed: {errors}")
