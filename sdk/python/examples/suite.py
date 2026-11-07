"""A test suite: one team over a set of tasks, each repeated, scored by the same checks; then the
pass rates, safety and spread. Optionally compare with an earlier suite.

    uv run python examples/suite.py [tasks (comma-separated, or "all")] [repeats] [model] [compare_with_suite_id]

Real model calls (Claude Code agents): every run costs credits. Start small.
"""

import sys

from common import connect, need_claude

need_claude()
tasks_arg = sys.argv[1] if len(sys.argv) > 1 else None
repeats = int(sys.argv[2]) if len(sys.argv) > 2 else 1
model = sys.argv[3] if len(sys.argv) > 3 else "haiku"
against = sys.argv[4] if len(sys.argv) > 4 else None

with connect() as arena:
    if tasks_arg in (None, ""):
        tasks = [arena.tasks()[0].id]
    elif tasks_arg == "all":
        tasks = None
    else:
        tasks = tasks_arg.split(",")
    suite = arena.suite(team=arena.default_team(model=model), tasks=tasks, repeats=repeats,
                        name=f"sdk example, {model}")
    print(f"suite {suite.id}: {len(suite.report['runs'])} runs, arenad runs them one after another ...")
    report = suite.wait()
    s = report["summary"]
    print(f"\n{report['status']}: {s['scored']} scored of {s['runs']}, pass rate {s['pass_rate']}, "
          f"score {s['score']}, safety violations {s['safety_violations']}")
    for c in report["checks"]:
        print(f"  task {c['task_id']:<8} {c['name']:<32} {c['passed']}/{c['runs']}"
              f"{' (safety)' if c['safety'] else ''}")
    for name in ("measured_s", "tokens", "cost_usd"):
        sp = s.get(name) or {}
        print(f"  {name:<12} n={sp.get('n')} min={sp.get('min')} mean={sp.get('mean')} max={sp.get('max')}")
    if against:
        cmp = arena.compare_suites(against, suite)
        print(f"\nagainst {against}: {cmp['verdict']} (pass rate {cmp['pass_rate']})")
    print(f"\nsuite id: {suite.id}")
