"""Counterfactual fork: go back to the first step that changed the world, give that agent another
prompt (or model), rerun from there with the world restored, and compare the two branches.

    uv run python examples/counterfactual_fork.py [branch_id] [model]

Without a branch id it first runs the benchmark's team (Claude Code) on the first task. Real model
calls: it costs credits. With ``ARENA_URL`` unset, the branch id must come from the same run of this
script (the arenad it starts stops at the end).
"""

import sys

from arena import Change
from common import connect, need_claude

need_claude()
model = sys.argv[2] if len(sys.argv) > 2 else "haiku"

with connect() as arena:
    if len(sys.argv) > 1 and sys.argv[1] != "-":
        original = arena.branch(sys.argv[1])
    else:
        original = arena.run(arena.tasks()[0].id, team=arena.default_team(model=model), seed=1)
        print(f"original branch {original.id}: running ...")
    rec = original.wait()
    print(f"original: {rec.status}")

    writes = [e for e in rec.tool_calls() if e.writes] or rec.tool_calls()
    if not writes:
        sys.exit("the original branch made no tool call to fork at")
    at = writes[0]
    agent = at.agent
    print(f"forking at step {at.seq}: {at.summary()}")
    print("restore plan:", original.restore_plan(at.seq)["parts"])

    prompt = next(a.prompt for a in rec.team.agents if a.id == agent)
    change = Change(prompt=prompt + "\n\nBefore any change, read back the order and confirm every item id "
                                    "against the customer's request. Never guess an id.")
    fork = original.fork(at.seq, changes={agent: change}, restore="exact")
    info = fork.fork_info
    print(f"fork {fork.id} starts at step {info.start_seq}"
          + (f" (moved from {info.moved_from}: {info.reason})" if info.moved_from else ""))
    for e in fork.stream():
        if e.seq >= info.start_seq:
            print(e.summary())

    cmp = arena.compare(original, fork)
    print(f"\noriginal {rec.status}, fork {fork.record().status}")
    print(f"first divergent step: {cmp.get('first_divergent_seq')}")
    print(f"state that differs at the end: {cmp.get('differing_rows')}")
