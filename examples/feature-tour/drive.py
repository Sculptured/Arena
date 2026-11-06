#!/usr/bin/env python3
"""Record deterministic pharma demos: scripted agents, illustrative usage, no models."""
import argparse
import json
from pathlib import Path

from behaviour import NAMES, TASKS, behaviour
from client import API, Calls, DISCLOSURE, TEAMS, deployment_team, disk_guard
from scenario import walk


def run(api, started, deployment, task, repeat, pace):
    api.request(f"/api/runs/{started['run_id']}/name", {
        "name": f"Demo: {NAMES[deployment]} / {task} / repeat {repeat}"})
    calls = Calls(api, started, deployment, pace, behaviour(deployment, task, repeat))
    try:
        walk(calls, task)
    except Exception as error:
        # Leave a scored failure with the reason, rather than an abandoned live branch.
        calls.end(str(error))
        raise
    result = calls.end()
    detail = api.request(f"/api/branches/{calls.branch}")
    audit = audit_branch(detail)
    expected = int(calls.behaviour.premature_write)
    if audit["refused_bookings"] != expected:
        raise RuntimeError(f"Expected {expected} refused bookings, got {audit}")
    started["audit"] = audit
    print(json.dumps({**started, "result": result}), flush=True)
    return result


def audit_branch(detail):
    events = detail["events"]
    calls = [e["payload"] for e in events if e["kind"] == "tool_call"]
    refused = [c for c in calls if c.get("name") == "book_shipment" and c.get("error")]
    reads = {t for group in ("erp_query", "crm_query", "reports", "lims")
             for t in TEAMS["tools"][group]} | {"shipments", "list_changes", "qualified_lanes", "customer_notices"}
    return {"tool_calls": len(calls), "read_calls": sum(c.get("name") in reads for c in calls),
            "refused_bookings": len(refused),
            "messages": sum(e["kind"] == "message" for e in events)}


def suites(api, pace, output):
    record = {"disclosure": DISCLOSURE, "suites": {}}
    for deployment, label, repeats in (("v1", "team v1", 2), ("v2", "team v2 (tuned)", 2),
                                        ("single", "single agent", 1)):
        suite = api.request("/api/suites", {"name": f"Demo: {label} — {DISCLOSURE}",
            "team": deployment_team(deployment), "tasks": TASKS, "repeats": repeats, "runner": "external"})
        sid = suite["id"]
        record["suites"][deployment] = {"id": sid, "runs": []}
        save(output, record)
        while True:
            started = api.request(f"/api/suites/{sid}/claim", {})
            if started["done"]:
                break
            record["suites"][deployment]["runs"].append(started)
            save(output, record)
            run(api, started, deployment, started["task_id"], started["repeat"], pace)
        record["suites"][deployment]["report"] = api.request(f"/api/suites/{sid}")
        save(output, record)
    a, b = (record["suites"][key]["id"] for key in ("v1", "v2"))
    record["compare"] = api.request(f"/api/suites/compare?a={a}&b={b}")
    save(output, record)
    verify(record)


def verify(record):
    reports = {key: row["report"] for key, row in record["suites"].items()}
    for key, count in (("v1", 10), ("v2", 10), ("single", 5)):
        summary = reports[key]["summary"]
        if summary["scored"] != count or summary["unscored"]:
            raise RuntimeError(f"{key}: incomplete scoring: {summary}")
    if reports["v1"]["summary"]["runs_with_violation"] != 1:
        raise RuntimeError("v1 must have exactly one run with a safety violation")
    for key in ("v2", "single"):
        if reports[key]["summary"]["pass_rate"] != 1:
            raise RuntimeError(f"{key} did not pass every check")
    if record["compare"]["verdict"] != "improved":
        raise RuntimeError("the v1-to-v2 comparison did not improve")


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("suites", "live"))
    parser.add_argument("--url", default="http://127.0.0.1:4513")
    parser.add_argument("--pace", type=float, default=0)
    parser.add_argument("--deployment", choices=NAMES, default="v1")
    parser.add_argument("--task", choices=TASKS, default=TASKS[0])
    parser.add_argument("--output", type=Path, default=Path("demo-results.json"))
    args = parser.parse_args()
    if args.pace < 0:
        parser.error("--pace must be nonnegative")
    disk_guard()
    api = API(args.url)
    if args.command == "suites":
        suites(api, args.pace, args.output)
    else:
        started = api.request("/api/runs", {"task_id": args.task, "runner": "external",
                                           "team": deployment_team(args.deployment)})
        save(args.output, started)
        run(api, started, args.deployment, args.task, 1, args.pace)


if __name__ == "__main__":
    main()
