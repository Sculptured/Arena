"""Export branches (a run, its forks, a suite) as a dataset folder for analysis.

The folder holds:

- ``events.jsonl``: every recorded event, one per line, with its run, branch and task;
- ``branches.jsonl``: one line per branch: how it started (task, seed, scenario, team, runner, fork
  point), how it ended (status, checks, score, safety violations) and its metrics;
- ``suite.json``: the suite's report, when a suite was exported;
- ``SCHEMA.md``: what every field means.

Load it with pandas: ``pd.read_json("events.jsonl", lines=True)``.
"""

from __future__ import annotations

import json
import os
from importlib import resources
from typing import TYPE_CHECKING, Any, Iterable, Optional

from .models import BranchRecord

if TYPE_CHECKING:
    from .client import Arena

FORMAT = "arena-dataset/1"


def branch_row(rec: BranchRecord) -> dict[str, Any]:
    """The ``branches.jsonl`` line of one branch."""
    start = rec.run_start
    check = rec.check
    return {
        "format": FORMAT,
        "run_id": rec.run_id,
        "branch_id": rec.id,
        "parent_id": rec.parent_id,
        "start_seq": rec.start_seq,
        "task_id": start.get("task_id"),
        "benchmark": start.get("benchmark"),
        "seed": start.get("seed"),
        "base_seed": start.get("base_seed"),
        "runner": start.get("runner"),
        "scenario": start.get("scenario"),
        "world": start.get("world"),
        "team": rec.team.to_dict(),
        "status": rec.status,
        "error": rec.error,
        "created_at": rec.created_at.isoformat() if rec.created_at else None,
        "ended_at": rec.ended_at.isoformat() if rec.ended_at else None,
        "restore_mode": rec.restore_mode,
        "restored": rec.restored,
        "fork_note": rec.fork_note,
        "passed": check.passed if check else None,
        "score": check.score if check else None,
        "checks": [c.__dict__ for c in check.checks] if check else None,
        "safety_violations": [c.name for c in check.safety_violations] if check else None,
        "differing_rows": check.differing_rows if check else None,
        "metrics": rec.metrics,
        "events": len(rec.events),
    }


def event_rows(rec: BranchRecord) -> Iterable[dict[str, Any]]:
    """The ``events.jsonl`` lines of one branch. A fork's record starts with its parent's events
    before the fork point; those are marked ``inherited``."""
    task = rec.run_start.get("task_id")
    for e in rec.events:
        yield {
            "run_id": rec.run_id,
            "branch_id": rec.id,
            "task_id": task,
            "seq": e.seq,
            "at": e.at.isoformat() if e.at else None,
            "agent": e.agent,
            "kind": e.kind,
            "inherited": rec.start_seq is not None and e.seq < rec.start_seq,
            "payload": e.payload,
        }


def schema_text() -> str:
    return resources.files("arena").joinpath("SCHEMA.md").read_text(encoding="utf-8")


def export_branches(
    arena: "Arena",
    branch_ids: Iterable[str],
    path: str,
    suite: Optional[dict[str, Any]] = None,
) -> str:
    """Writes the branches as a dataset folder at ``path`` (made if missing; its files are
    replaced). Returns ``path``."""
    os.makedirs(path, exist_ok=True)
    with open(os.path.join(path, "events.jsonl"), "w", encoding="utf-8") as ev, open(
        os.path.join(path, "branches.jsonl"), "w", encoding="utf-8"
    ) as br:
        for bid in branch_ids:
            rec = BranchRecord.from_dict(arena.get(f"/api/branches/{bid}"))
            br.write(json.dumps(branch_row(rec), default=str) + "\n")
            for row in event_rows(rec):
                ev.write(json.dumps(row, default=str) + "\n")
    if suite is not None:
        with open(os.path.join(path, "suite.json"), "w", encoding="utf-8") as f:
            json.dump(suite, f, indent=2, default=str)
    with open(os.path.join(path, "SCHEMA.md"), "w", encoding="utf-8") as f:
        f.write(schema_text())
    return path


def read_jsonl(path: str) -> list[dict[str, Any]]:
    """Reads one of the dataset's ``.jsonl`` files."""
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]
