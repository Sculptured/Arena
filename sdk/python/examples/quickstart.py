"""Quickstart: run a team of Claude Code agents on a task, watch the timeline live, print the checks.

    uv run python examples/quickstart.py [task_id] [model]

Real model calls (Claude Code, default model haiku): it costs credits.
"""

import sys

from common import connect, need_claude, task_arg

need_claude()
model = sys.argv[2] if len(sys.argv) > 2 else "haiku"

with connect() as arena:
    bench = arena.benchmark()
    task = task_arg(arena)
    team = arena.default_team(model=model)          # the benchmark's suggested team, every agent on `model`
    print(f"{bench.name} {bench.version}, task {task}, team: "
          + ", ".join(f"{a.id} ({a.kind} {a.model})" for a in team.agents))

    branch = arena.run(task, team=team, seed=1)       # arenad runs the agents one after another
    print(f"run {branch.run_id}, branch {branch.id}\n")
    for event in branch.stream():                    # live; ends when the branch has been scored
        print(event.summary())

    rec = branch.record()
    check = rec.check
    print(f"\n{rec.status}: {rec.error or ''}")
    if check:
        for c in check.checks:
            print(f"  {'PASS' if c.passed else 'FAIL'} {c.name}{' (safety)' if c.safety else ''} {c.detail}")
        if check.differing_rows:
            print("  differs from the expected world:", ", ".join(check.differing_rows[:10]))
    m = rec.metrics
    print(f"measured {m.get('measured_s')} s, estimated in production {m.get('estimated_s')} s, "
          f"tokens {m.get('tokens')}, cost {m.get('cost', {}).get('usd')} ({m.get('cost', {}).get('label')})")
    print(f"\nbranch id for the other examples: {branch.id}")
