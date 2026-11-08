"""Writes the shot list for capture.mjs from the showcase's metrics.json and Arena's API: the runs
list, each design's first run (its metrics and estimates, and one telling step in the inspector),
Arena's compare between the designs, and a fork's panel. Read only.

    python shots.py metrics.json > shots.json
"""
import json
import sys
import urllib.request
from pathlib import Path

BASE = "http://127.0.0.1:4510"


def branch(bid: str) -> dict:
    with urllib.request.urlopen(f"{BASE}/api/branches/{bid}", timeout=120) as r:
        return json.loads(r.read())


def size(e: dict) -> int:
    return len(json.dumps(e["payload"].get("result")))


def telling_step(events: list) -> int | None:
    """A message between agents when there is one (the design at work), else the largest tool
    result (the context one agent has to carry)."""
    calls = [e for e in events if e["kind"] == "tool_call"]
    talk = [e for e in events if e["kind"] == "message" or
            (e["kind"] == "tool_call" and e["payload"].get("name") in ("message", "handoff"))]
    purchases = [e for e in calls if e["payload"].get("name") == "submit_doc" and
                 e["payload"].get("args", {}).get("doctype") == "Purchase Order"]
    pick = talk[len(talk) // 3] if talk else next(iter(purchases), None) or max(calls, key=size, default=None)
    return pick and pick["seq"]


def run_shots(key: str, bid: str) -> list:
    b = branch(bid)
    path = f"/runs/{b['run_id']}/{bid}"
    shots = [{"name": f"{key}-run", "path": path, "settle_ms": 2500,
              "record": True, "scroll": 1400, "record_s": 7}]
    step = telling_step(b["events"])
    if step is not None:
        shots[0]["actions"] = [
            {"focus": ".usage-chart", "hold_s": 5},
            {"focus": ".activity-overview", "hold_s": 6},
            {"activate": f'.activity-tick[aria-label*="step {step}:"]', "hold_s": 3},
            {"focus": ".insp-actions", "hold_s": 9},
        ]
        shots.append({"name": f"{key}-step", "path": f"{path}?step={step}", "focus": ".inspector", "settle_ms": 2500})
    return shots


def main() -> None:
    m = json.load(open(sys.argv[1]))
    first = {k: d["runs"][0]["branch"] for k, d in m["designs"].items()}
    shots = [{"name": "runs", "path": "/", "settle_ms": 2000}]
    keys = list(first)
    for k, bid in first.items():
        shots += run_shots(k, bid)
    metrics_branch = branch(first[keys[0]]) if keys else None
    if metrics_branch:
        shots.append({"name": "arena-metrics", "path": f"/runs/{metrics_branch['run_id']}/{metrics_branch['id']}",
                      "wait": ".usage-chart", "hover": ".metrics", "settle_ms": 2000,
                      "record": True, "actions": [
                          {"focus": ".metrics summary", "hold_s": 6},
                          {"focus": ".usage-chart", "hold_s": 8},
                          {"focus": ".metrics table:last-of-type", "hold_s": 8}]})
    for a, b in zip(keys, keys[1:] + keys[:1]):
        shots.append({"name": f"compare-{a}-{b}", "path": f"/compare/{first[a]}/{first[b]}",
                      "settle_ms": 2500, "record": True, "scroll": 1200, "record_s": 6,
                      "timeout_ms": 300000, "warm": f"/api/compare?a={first[a]}&b={first[b]}"})
    fork_of = first[keys[-1]]
    fork_branch = branch(fork_of)
    fork_step = telling_step(fork_branch["events"])
    if fork_step is not None:
        shots.append({"name": "fork-panel", "path": f"/runs/{fork_branch['run_id']}/{fork_of}?step={fork_step}&fork=1",
                      "wait": ".fork-panel", "focus": ".fork-head", "settle_ms": 2500})
    for name, view in [("checks", "Outcomes"), ("milestones", "Timing"), ("context", "Context"),
                       ("messages", "Coordination"), ("cost", "Deployment")]:
        shots.append({"name": name, "path": f"/experiment?view={view}",
                      "upload": {"selector": ".report-upload", "path": str(Path(sys.argv[1]).resolve())},
                      "wait": ".experiment-grid", "record": True,
                      "actions": [{"focus": ".experiment-grid", "hold_s": 8},
                                  {"focus": ".experiment-design:first-child .evidence-link", "hold_s": 8}]})
    print(json.dumps({"base": BASE, "width": 1536, "height": 720, "scale": 1.25, "shots": shots}, indent=1))


if __name__ == "__main__":
    main()
