"""The disruption a task's setup may schedule in the lab (arena_part.disruptions): a lot's release
test fails, so the lot can't be released."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import lab


def _check(config, do) -> None:
    lab.lot(config, do["lot"])
    if do["test"] not in config["lims"]["test_names"]:
        raise Refusal(f"no test {do['test']!r} (tests: {', '.join(config['lims']['test_names'])})")


def _fails(config, state, do) -> list:
    state.setdefault("failed", {})[do["lot"]] = do["test"]
    return [f"batch/{do['lot']}"]


KINDS = {"result_fails": Kind({"lot": "string", "test": "string"}, _fails, _check)}
