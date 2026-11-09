"""The Line 2 OEE benchmark: one simulation part in its world, one task (reach a target OEE within a
simulation budget and the plant's limits), three suggested optimiser designs, scored by named checks
on the re-simulated recommendation, drawn as a twin."""
import json
from pathlib import Path

from arena_part import Benchmark, errors

from . import checks, setups, twin

VERSION = "0.1.0"
DATA = Path(__file__).resolve().parents[1] / "data" / "plant.json"
SIM = "line-sim"


def load(path: Path = DATA) -> dict:
    return json.loads(path.read_text())


class Line2Oee(Benchmark):
    def __init__(self, config: dict | None = None):
        self.config = config if config is not None else load()

    def describe(self):
        task = {"id": setups.TASK, "reason": "Line 2 from 77% to 85% OEE, no extra headcount",
                "ticket": setups.TICKET}
        return {"name": "line2-oee", "version": VERSION, "protocol": 1,
                "world": [{"part": "sim", "integration": SIM, "config": self.config}],
                "tasks": [task], "instructions": setups.INSTRUCTIONS,
                "instructions_title": setups.INSTRUCTIONS_TITLE,
                "team": setups.SINGLE, "teams": [setups.TEAM, setups.STRATEGIST], "tools": [],
                "scored": True, "view": {"docs": ["sim"]}}

    def score(self, task, docs):
        self._task(task)
        return {"checks": checks.evaluate(docs)}

    def view(self, connections, docs):
        return {"view": twin.view(self.config, docs)}

    @staticmethod
    def _task(task):
        if task != setups.TASK:
            raise errors.invalid(f"unknown task {task}")
