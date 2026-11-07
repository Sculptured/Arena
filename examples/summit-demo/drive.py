#!/usr/bin/env python3
"""Drive the summit demo on the demo arenad: scripted agents, illustrative usage, no model calls.

  live       start a paced run of Team v1 (or another deployment) with a disruption, and drive it
  watch      drive every run an external system is awaited for: runs started from the UI's New run
             dialog ("by an external system") and forks made with Rewind here, each by the
             deployment its team is
  fork       fork a branch at a step with another deployment and resume it from there
  duplicate  the duplicate-shipment preset: an escalated order, two desks (v1 books it twice)
  batch      a suite of seeded incidents (tasks x disruption params x repeats) for one deployment
  reset      delete the rehearsal runs (the batch suites stay) and check the demo is ready
  world      record the reference run the World beat opens (Team v2, no disruption) and print its URL
"""
import argparse
import json
import shutil
import sys
import time
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

from calls import DemoCalls, Divergence, ResumingCalls, recorded_calls
from client import API, disk_guard
from decisions import DEPLOYMENTS, NAMES, TASKS, behaviour, deployment_of
from incident import walk
from teams import DISCLOSURE, fork_body, team

URL = "http://127.0.0.1:4513"
STATE = Path("/home/alexsky2/sculptured-work-local/summit-demo")
CLAIMS = STATE / "claims"
MIN_FREE_GB = 21
LIVE_PREFIX = "Summit live"
WORLD_PREFIX = "Summit world"
# The batch: every task, in each of these disruption scenarios, twice (v1's mistake differs by repeat).
BATCH_SCENARIOS = [
    {"disruptions": [{"id": "lane-unavailable", "params": {"carrier": "Tundra Medical Courier", "after": 2}}]},
    {"disruptions": [{"id": "transit-delay", "params": {"carrier": "Tundra Medical Courier", "days": 2, "after": 0}}]},
    {"disruptions": [{"id": "retest-fails", "params": {"lot": "CVX-2608", "after": 2}}]},
]
DUPLICATE = {"disruptions": [{"id": "urgent-escalation", "params": {"order": "PO-SBMC-4471", "after": 0}}]}


def claim(branch):
    """Marks a branch as driven by this driver (once): False when someone already claimed it."""
    CLAIMS.mkdir(parents=True, exist_ok=True)
    try:
        (CLAIMS / branch).open("x").close()
        return True
    except FileExistsError:
        return False


def run_start(detail):
    return next(e["payload"] for e in detail["events"] if e["kind"] == "run_start")


def finish(c, error=None):
    """Ends every agent with its illustrative usage and finishes the branch: how it ended."""
    return c.end(error)


def drive(api, c, task):
    try:
        walk(c, task)
    except Exception as error:
        finish(c, f"{type(error).__name__}: {error}")
        raise
    return finish(c)


def name_run(api, run_id, name):
    api.request(f"/api/runs/{run_id}/name", {"name": name})


def scenario_arg(text):
    """`id` or `id:name=value,name=value` as one scenario disruption (ints stay ints)."""
    if not text:
        return None
    did, _, rest = text.partition(":")
    params = {}
    for pair in filter(None, rest.split(",")):
        k, _, v = pair.partition("=")
        params[k] = int(v) if v.lstrip("-").isdigit() else v
    return {"disruptions": [{"id": did, "params": params}]}


def start_run(api, task, deployment, scenario, expediting=False):
    body = {"task_id": task, "runner": "external", "team": team(deployment, expediting)}
    if scenario:
        body["scenario"] = scenario
    return api.request("/api/runs", body)


def live(api, args):
    started = start_run(api, args.task, args.deployment, scenario_arg(args.disruption))
    claim(started["branch_id"])
    what = f"{NAMES[args.deployment]} / {args.task}" + (f" / {args.disruption}" if args.disruption else "")
    name_run(api, started["run_id"], f"{LIVE_PREFIX}: {what} · {DISCLOSURE}")
    print(f"run {started['run_id']} branch {started['branch_id']}", flush=True)
    print(f"world: {api.url}/world/{started['run_id']}/{started['branch_id']}", flush=True)
    c = DemoCalls(api, started, behaviour(args.deployment, "live"), paced=not args.fast)
    print(json.dumps(drive(api, c, args.task)))
    return started


def register_team(api, branch, agents):
    """Starts each of the branch's agents again (its AgentStart on the new branch)."""
    for a in agents:
        api.request(f"/api/branches/{branch}/agents", {k: a.get(k) for k in
                    ("id", "name", "kind", "model", "prompt", "read_only")})


def resume(api, branch, paced=True):
    """Drives a fork of an external run from its fork point, by the deployment its team is."""
    detail = api.request(f"/api/branches/{branch}")
    task = run_start(detail)["task_id"]
    deployment = deployment_of(detail["team"])
    register_team(api, branch, detail["team"]["agents"])
    record = recorded_calls(detail["events"], detail["start_seq"])
    started = {"run_id": detail["run_id"], "branch_id": branch}
    c = ResumingCalls(api, started, behaviour(deployment, "live"), paced, record)
    print(f"resuming {branch} from step {detail['start_seq']} as {NAMES[deployment]}", flush=True)
    try:
        return drive(api, c, task)
    except Divergence as error:
        print(f"diverged: {error}", file=sys.stderr)
        raise


def fork(api, args):
    parent = api.request(f"/api/branches/{args.branch}")
    body = fork_body(parent["team"], args.at, args.deployment)
    forked = api.request(f"/api/branches/{args.branch}/fork", body)
    claim(forked["branch_id"])
    print(f"fork {forked['branch_id']} starts at step {forked['start_seq']}", flush=True)
    print(json.dumps(resume(api, forked["branch_id"], paced=not args.fast)))
    print(f"compare: {api.url}/compare/{args.branch}/{forked['branch_id']}")
    return forked


def waiting(detail):
    """True for a live branch of an external run that no agent has acted on yet (its own events)."""
    if detail["status"] != "running" or run_start(detail).get("runner") != "external":
        return False
    since = detail.get("start_seq") or 0
    acted = [e for e in detail["events"] if e["seq"] >= since and e["agent"] not in (None, "link", "setup")
             and e["kind"] in ("tool_call", "model_call", "message", "agent_end")]
    created = datetime.fromisoformat(detail["created_at"].replace("Z", "+00:00"))
    return not acted and (datetime.now(timezone.utc) - created).total_seconds() > 2


def drive_waiting(api, detail, paced):
    branch = detail["id"]
    if detail.get("start_seq") is not None:
        return resume(api, branch, paced)
    deployment = deployment_of(detail["team"])
    started = {"run_id": detail["run_id"], "branch_id": branch}
    print(f"driving {branch} as {NAMES[deployment]}", flush=True)
    c = DemoCalls(api, started, behaviour(deployment, "live"), paced)
    return drive(api, c, run_start(detail)["task_id"])


def watch(api, args):
    print("watching for runs and forks to drive (Ctrl-C stops)", flush=True)
    while True:
        for run in api.request("/api/runs"):
            for b in (x for x in run["branches"] if x["status"] == "running"):
                detail = api.request(f"/api/branches/{b['id']}")
                if waiting(detail) and claim(b["id"]):
                    try:
                        print(json.dumps(drive_waiting(api, detail, not args.fast)), flush=True)
                    except Exception as error:  # keep watching: the branch ended with the reason
                        print(f"{b['id']}: {error}", file=sys.stderr, flush=True)
        time.sleep(1)


def duplicate(api, args):
    started = start_run(api, "cvx-2611-em-excursion", args.deployment, DUPLICATE, expediting=True)
    claim(started["branch_id"])
    name_run(api, started["run_id"], f"Summit duplicate: {NAMES[args.deployment]} · {DISCLOSURE}")
    c = DemoCalls(api, started, behaviour(args.deployment, "duplicate"), paced=False)
    print(json.dumps({**started, "result": drive(api, c, "cvx-2611-em-excursion")}))
    return started


def free_gb():
    return shutil.disk_usage("/").free / 1e9


def batch(api, args):
    """A new batch suite, or with --resume the one recorded before (its unclaimed runs). Stops
    before a run when the disk has less than MIN_FREE_GB free."""
    STATE.mkdir(parents=True, exist_ok=True)
    out = STATE / f"batch-{args.deployment}.json"
    if args.resume:
        record = json.loads(out.read_text())
        suite = {"id": record["suite"]}
    else:
        suite = api.request("/api/suites", {
            "name": f"Summit batch: {NAMES[args.deployment]} · {DISCLOSURE}", "team": team(args.deployment),
            "tasks": list(TASKS), "repeats": args.repeats, "runner": "external", "scenarios": BATCH_SCENARIOS})
        record = {"suite": suite["id"], "runs": []}
    out.write_text(json.dumps(record, indent=2))
    while free_gb() >= MIN_FREE_GB and not (started := api.request(f"/api/suites/{suite['id']}/claim", {}))["done"]:
        claim(started["branch_id"])
        name_run(api, started["run_id"], f"Summit batch: {NAMES[args.deployment]} / {started['task_id']} / "
                 f"scenario {started['variant'] + 1} / repeat {started['repeat']} · {DISCLOSURE}")
        c = DemoCalls(api, started, behaviour(args.deployment, "batch", started["repeat"]), paced=False)
        try:
            started["result"] = drive(api, c, started["task_id"])
        except Exception as error:
            started["result"] = {"driver_error": str(error)}
        record["runs"].append(started)
        out.write_text(json.dumps(record, indent=2))
    record["report"] = api.request(f"/api/suites/{suite['id']}")["summary"]
    out.write_text(json.dumps(record, indent=2))
    print(json.dumps({"suite": suite["id"], "summary": record["report"]}, indent=2))


def delete(api, path):
    req = urllib.request.Request(api.url + path, method="DELETE")
    with urllib.request.urlopen(req, timeout=600) as r:
        return r.status


def reset(api, args):
    """Deletes the rehearsals (live, duplicate and UI-started external runs outside any suite) and
    lists what the demo starts from."""
    in_suites = {r["run_id"] for s in api.request("/api/suites")
                 for r in api.request(f"/api/suites/{s['id']}")["runs"] if r["run_id"]}
    for run in api.request("/api/runs"):
        name = run.get("name") or ""
        rehearsal = name.startswith((LIVE_PREFIX, WORLD_PREFIX, "Summit duplicate")) or (
            not name and run["id"] not in in_suites and run["branches"][0]["team"]["agents"][0]["kind"] == "external")
        if rehearsal and run["id"] not in in_suites:
            print(f"deleting {run['id']} {name or '(UI rehearsal)'}: {delete(api, '/api/runs/' + run['id'])}")
    for f in CLAIMS.glob("*"):
        f.unlink()
    suites = [s for s in api.request("/api/suites") if s["name"].startswith("Summit batch")]
    for s in suites:
        print(f"batch suite {s['id']}: {s['name']} ({s['ended']}/{s['runs']} ended, {s['passed']} passed)")
    print(f"ready: {api.url}/  (benchmark {api.request('/api/benchmark')['name']})")


def world(api, args):
    """The World beat's reference: Larchmont as every run starts, then a correct response to it."""
    started = start_run(api, TASKS[0], "v2", None)
    claim(started["branch_id"])
    name_run(api, started["run_id"], f"{WORLD_PREFIX}: Larchmont Biologics, CVX-2611 on hold · {DISCLOSURE}")
    drive(api, DemoCalls(api, started, behaviour("v2", "live"), paced=False), TASKS[0])
    print(f"World beat: {api.url}/world/{started['run_id']}/{started['branch_id']}?step=1")
    return started


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("command", choices=("live", "watch", "fork", "duplicate", "batch", "reset", "world"))
    p.add_argument("--url", default=URL)
    p.add_argument("--task", choices=TASKS, default=TASKS[0])
    p.add_argument("--deployment", choices=DEPLOYMENTS, default="v1")
    p.add_argument("--disruption", default="courier-capacity:after=1,capacity=0",
                   help="id[:name=value,...] of a scenario option ('' for none)")
    p.add_argument("--branch", help="fork: the branch to fork")
    p.add_argument("--at", type=int, help="fork: the step to fork at")
    p.add_argument("--repeats", type=int, default=2)
    p.add_argument("--fast", action="store_true", help="no pacing")
    p.add_argument("--resume", action="store_true", help="batch: go on with the suite recorded before")
    args = p.parse_args()
    if args.command == "fork" and (args.branch is None or args.at is None):
        p.error("fork needs --branch and --at")
    disk_guard()
    api = API(args.url)
    {"live": live, "watch": watch, "fork": fork, "duplicate": duplicate, "batch": batch,
     "reset": reset, "world": world}[args.command](api, args)


if __name__ == "__main__":
    main()
