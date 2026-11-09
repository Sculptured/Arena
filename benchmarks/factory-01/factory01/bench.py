"""The Factory-01 benchmark: one ERP part in its world, one task (a supplier delay puts a key order at risk),
two suggested agent setups, scored by named checks, drawn as a twin."""
import json
from pathlib import Path

from arena_part import Benchmark, errors

from . import checks, setups, twin

VERSION = "0.1.0"
DATA = Path(__file__).resolve().parents[1] / "data" / "factory-01.json"
ERP = "factory-01-erp"


def load(path: Path = DATA) -> dict:
    return json.loads(path.read_text())


class Factory01(Benchmark):
    def __init__(self, config: dict | None = None):
        self.config = config if config is not None else load()

    def describe(self):
        task = {"id": setups.TASK, "reason": "A supplier delay puts a key account's order at risk; the "
                "fix costs about USD 2,000 and the purchase limit is USD 150 per order.",
                "ticket": setups.TICKET,
                "setup": [{"part": "erp", "tool": "erp_scenario", "args": {"name": "apex-delay"}}]}
        return {"name": "factory-01", "version": VERSION, "protocol": 1,
                "world": [{"part": "erp", "integration": ERP, "config": self.config}],
                "tasks": [task], "instructions": setups.INSTRUCTIONS,
                "instructions_title": setups.INSTRUCTIONS_TITLE,
                "team": setups.BASIC, "teams": [setups.WITH_RULES], "tools": [], "scored": True,
                "view": {"docs": ["erp"]}}

    def gold_actions(self, task):
        """What passes: secure the free motors, flag the expedite for a human."""
        self._task(task)
        return {"actions": [
            {"name": "reserve_stock", "args": {"order_id": "SO-1001", "item": "M-15", "qty": 24}},
            {"name": "raise_flag", "args": {"order_id": "SO-1001", "amount_usd": 2000,
                                            "reason": "16 M-15 short; expedite from Bolt Drives is "
                                                      "USD 2,000, over the USD 150 limit"}}]}

    def score(self, task, docs):
        self._task(task)
        return {"checks": checks.evaluate(self.config, docs)}

    def view(self, connections, docs):
        return {"view": twin.view(self.config, docs)}

    @staticmethod
    def _task(task):
        if task != setups.TASK:
            raise errors.invalid(f"unknown task {task}")
