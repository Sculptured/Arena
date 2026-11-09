"""The plant-floor benchmark: an ERP (orders, suppliers, purchasing with a spend cap) and Plant 2's floor over
OPC UA; one task (a supplier fire, a rush order and a robot cell down land on Monday morning); three agent
designs; scored by named checks; drawn as a twin."""
import json
from pathlib import Path

from arena_part import Benchmark, errors

from . import checks, setups, twin

VERSION = "0.1.0"
DATA = Path(__file__).resolve().parents[1] / "data" / "plant-floor.json"


def load(path: Path = DATA) -> dict:
    return json.loads(path.read_text())


class PlantFloor(Benchmark):
    def __init__(self, config=None):
        self.config = config if config is not None else load()

    def describe(self):
        task = {"id": setups.TASK, "reason": "A supplier fire, a key-account rush order and a faulted robot cell land "
                "on the plant floor on Monday morning; re-plan the week over OPC UA.", "ticket": setups.TICKET,
                "setup": [{"part": "erp", "tool": "erp_scenario", "args": {"name": "apex-fire"}},
                          {"part": "plant", "tool": "plant_scenario", "args": {"name": "disrupted"}}]}
        return {"name": "plant-floor", "version": VERSION, "protocol": 1,
                "world": [{"part": "erp", "integration": "factory-01-erp", "config": self.config["erp"]},
                          {"part": "plant", "integration": "plant-opcua", "config": self.config["plant"]}],
                "tasks": [task], "instructions": setups.INSTRUCTIONS, "instructions_title": setups.INSTRUCTIONS_TITLE,
                "team": setups.SINGLE_AGENT, "teams": [setups.TEAM, setups.TEAM_RULES], "tools": [],
                "scored": True, "view": {"docs": ["erp", "plant"]}}

    def score(self, task, docs):
        self._task(task)
        return {"checks": checks.evaluate(docs)}

    def view(self, connections, docs):
        return {"view": twin.view(self.config, docs)}

    @staticmethod
    def _task(task):
        if task != setups.TASK:
            raise errors.invalid(f"unknown task {task}")
