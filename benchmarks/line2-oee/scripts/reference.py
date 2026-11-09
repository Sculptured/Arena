"""The reference number: a plain random search with the agents' budget, run offline. Not a run, not an
agent: it shows what 30 simulations buy without any reasoning.

Each search simulates today's config first, then configs drawn uniformly within the decisions'
bounds, skipping any a hard limit would deny (an agent's denied call costs no simulation either): 30
in all, with 5 replications, through the same part and tools the agents use. It submits the best
feasible one, which the part verifies on 20 new replications. Several searches (seeds) give the spread. Writes `data/reference.json`.

    cd benchmarks/line2-oee && uv run python scripts/reference.py [searches]
"""
import json
import random
import statistics
import sys
import tempfile
from datetime import date
from pathlib import Path

HERE = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(HERE.parents[1] / "integrations" / "line-sim")]

from line_sim import plant  # noqa: E402
from line_sim.part import LineSim  # noqa: E402

CONFIG = json.loads((HERE / "data" / "plant.json").read_text())
REPS = 5


def draw(rng: random.Random, d: dict):
    if d["kind"] == "int_list":
        return [rng.randint(d["min"], d["max"]) for _ in range(d["len"])]
    if d["kind"] == "int":
        return rng.randint(d["min"], d["max"])
    return round(rng.uniform(d["min"], d["max"]) * 2) / 2


def candidate(rng: random.Random) -> dict:
    p = {k: v for k, v in CONFIG.items() if k != "study"}
    while True:
        config = {d["name"]: draw(rng, d) for d in CONFIG["decisions"]}
        if not plant.violations(p, config):
            return config


def search(seed: int) -> dict:
    rng = random.Random(seed)
    part = LineSim()
    part.start("i1", CONFIG, 0, 0)
    for i in range(CONFIG["study"]["max_simulations"]):
        config = candidate(rng) if i else {}
        out = part.call_tool("i1", "run_simulation", {"config": config, "replications": REPS}, 0)
        assert "error" not in out, out
    best = part.call_tool("i1", "get_history", {}, 0)["result"]["best_feasible"]
    if best is None:
        return {"seed": seed, "best": None, "evidence": None, "verified": None, "feasible": False}
    rec = part.call_tool("i1", "submit_recommendation", {"simulation": best, "summary": "random search"}, 0)
    r = rec["result"]
    return {"seed": seed, "best": best, "evidence": r["evidence"]["value"],
            "verified": r["verified"]["objective"]["value"], "feasible": r["verified"]["feasible"]}


def main(n: int) -> None:
    runs = [search(seed) for seed in range(n)]
    verified = [r["verified"] if r["feasible"] else 0.0 for r in runs]
    out = {"label": f"Random search, {CONFIG['study']['max_simulations']} simulations",
           "method": __doc__.split("\n\n")[1].replace("\n", " "),
           "searches": n, "replications": REPS, "made": date.today().isoformat(),
           "verified": {"median": round(statistics.median(verified), 1), "min": round(min(verified), 1),
                        "max": round(max(verified), 1)},
           "reached_target": sum(v >= 85 for v in verified), "runs": runs}
    (HERE / "data" / "reference.json").write_text(json.dumps(out, indent=2) + "\n")
    print(json.dumps(out["verified"]), out["reached_target"], "of", n, "reached 85%")


if __name__ == "__main__":
    import os
    os.environ.setdefault("ARENA_SAVES", tempfile.mkdtemp())
    main(int(sys.argv[1]) if len(sys.argv) > 1 else 10)
