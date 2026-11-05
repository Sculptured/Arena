"""Writes the shot list for capture.mjs from the showcase's metrics.json and Arena's API: the runs
list, each design's first run (its metrics and estimates, and one telling step in the inspector),
Arena's compare between the designs, and a fork's panel. Read only.

    python shots.py metrics.json > shots.json
"""
import json
import sys
import urllib.request

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
    talk = [e for e in calls if e["payload"].get("name") in ("message", "handoff")]
    pick = talk[len(talk) // 3] if talk else max(calls, key=size, default=None)
    return pick and pick["seq"]


def run_shots(key: str, bid: str) -> list:
    b = branch(bid)
    path = f"/runs/{b['run_id']}/{bid}"
    shots = [{"name": f"{key}-run", "path": path, "settle_ms": 2500,
              "record": True, "scroll": 1400, "record_s": 7}]
    step = telling_step(b["events"])
    if step is not None:
        shots.append({"name": f"{key}-step", "path": f"{path}?step={step}", "settle_ms": 2500})
    return shots


def main() -> None:
    m = json.load(open(sys.argv[1]))
    first = {k: d["runs"][0]["branch"] for k, d in m["designs"].items()}
    shots = [{"name": "runs", "path": "/", "settle_ms": 2000}]
    for k, bid in first.items():
        shots += run_shots(k, bid)
    keys = list(first)
    for a, b in zip(keys, keys[1:] + keys[:1]):
        shots.append({"name": f"compare-{a}-{b}", "path": f"/compare/{first[a]}/{first[b]}",
                      "settle_ms": 2500, "record": True, "scroll": 1200, "record_s": 6,
                      "timeout_ms": 300000, "warm": f"/api/compare?a={first[a]}&b={first[b]}"})
    fork_of = first[keys[-1]]
    shots.append({"name": "fork-panel", "path": f"/runs/{branch(fork_of)['run_id']}/{fork_of}?fork=1",
                  "settle_ms": 2500})
    print(json.dumps({"base": BASE, "width": 1280, "height": 720, "scale": 1.5, "shots": shots}, indent=1))


if __name__ == "__main__":
    main()
