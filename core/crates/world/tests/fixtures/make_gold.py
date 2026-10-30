"""Writes gold_changes.json: for every eligible retail task, the rows that differ from
base after applying the gold actions with the Python reference tools.

Run:  python3 -I make_gold.py
Eligible (as gap.select): at least one write action and no nl_assertions.
"""
import copy
import json
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / "../../../../../data/retail"
HARNESS = HERE / "../../../../../../cs2680-gap-tests/harness"
sys.path.insert(0, str(HARNESS.resolve()))
import tool  # noqa: E402


def gold_db(base, task):
    # A failing gold action leaves the database unchanged, as upstream (as gap.gold_db).
    db = copy.deepcopy(base)
    wrote = False
    for action in task["evaluation_criteria"]["actions"]:
        _, error, _, writes = tool.call(db, action["name"], action["arguments"])
        wrote = wrote or (error is None and bool(writes))
    if not wrote:
        raise RuntimeError(f"task {task['id']}: no gold write succeeded")
    return db


def main():
    base = json.load(open(DATA / "db.json"))
    tasks = json.load(open(DATA / "tasks.json"))
    out = {}
    for task in tasks:
        actions = task["evaluation_criteria"]["actions"] or []
        if not any(a["name"] in tool.WRITE_TOOLS for a in actions):
            continue
        if task["evaluation_criteria"].get("nl_assertions"):
            continue
        db = gold_db(base, task)
        changed = {}
        for table in ("users", "orders", "products"):
            for rid, doc in db[table].items():
                if doc != base[table][rid]:
                    changed[f"{table}/{rid}"] = doc
        out[task["id"]] = changed
    (HERE / "gold_changes.json").write_text(json.dumps(out, sort_keys=True))
    print(len(out), "tasks")


if __name__ == "__main__":
    main()
