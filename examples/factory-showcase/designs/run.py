"""Runs one design on Arena, or a fork of a recorded run with a changed instruction.

    python run.py --design A --out DIR [--model claude-sonnet-4-5] [--max-minutes 120]
    python run.py --design C --out DIR --fork-of BRANCH --at-seq N --change incident=prompt.txt
"""
import argparse
import json
import os
import threading
import time

import arena_api
from claude_agent import Agent
from designs import DESIGNS
from engine import Engine

TASK = "press-a1-bearing"


def first_message(prompt: str, task: dict) -> str:
    return (f"{prompt}\n\n# The ticket\n\n{task['ticket']}\n\n# Incident rules\n\n"
            f"{task['instructions']}")


def team_of(design, model: str) -> dict:
    agents = [{"id": r.id, "name": r.name, "kind": "claude_code", "model": model, "prompt": r.prompt}
              for r in design.roles]
    return {"id": f"design-{design.key.lower()}", "name": design.label, "agents": agents}


def history_note(history: list) -> str:
    """What a resumed agent is told of its own recorded work before the fork point."""
    lines = []
    for e in history:
        p = e.get("payload") or {}
        if e["kind"] == "tool_call":
            res = json.dumps(p.get("result") if p.get("error") is None else {"error": p.get("error")})
            lines.append(f"- called {p.get('name')} {json.dumps(p.get('args'))} -> {res[:600]}")
        elif e["kind"] in ("message", "handoff"):
            lines.append(f"- {e['kind']} from {e.get('agent')} to {p.get('to')}: {p.get('text') or p.get('note')}")
    if not lines:
        return ""
    return "\n\n# Your work so far, as Arena recorded it\n\n" + "\n".join(lines)


def open_log(out: str):
    f = open(os.path.join(out, "engine.jsonl"), "a")
    lock = threading.Lock()

    def log(rec: dict):
        rec["t"] = round(time.time(), 2)
        with lock:
            f.write(json.dumps(rec) + "\n")
            f.flush()
    return log


def start(args, design, task):
    """A new run, or a fork: (run id, branch id, its first own seq, {agent: (mcp_url, prompt,
    history)})."""
    if not args.fork_of:
        r = arena_api.start_run(TASK, team_of(design, args.model))
        eps = {a["agent"]: a["mcp_url"] for a in r["agents"]}
        return r["run_id"], r["branch_id"], 0, {x.id: (eps[x.id], x.prompt, []) for x in design.roles}
    changes = {}
    for c in args.change:
        agent, path = c.split("=", 1)
        changes[agent] = {"prompt": open(path).read().strip()}
    f = arena_api.fork(args.fork_of, args.at_seq, changes)
    agents = {a["agent"]: (a["mcp_url"], a["spec"]["prompt"], a["history"]) for a in f["agents"]}
    return f.get("run_id"), f["branch_id"], f["start_seq"], agents


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--design", required=True, choices=sorted(DESIGNS))
    ap.add_argument("--out", required=True)
    ap.add_argument("--model", default="claude-sonnet-4-5")
    ap.add_argument("--context-window", type=int, default=200000)
    ap.add_argument("--max-minutes", type=float, default=120)
    ap.add_argument("--fork-of")
    ap.add_argument("--at-seq", type=int)
    ap.add_argument("--change", action="append", default=[])
    args = ap.parse_args()
    os.makedirs(args.out, exist_ok=True)
    design, task = DESIGNS[args.design](), arena_api.task(TASK)
    run_id, branch, from_seq, specs = start(args, design, task)
    meta = {"design": design.key, "label": design.label, "model": args.model, "run_id": run_id,
            "branch_id": branch, "fork_of": args.fork_of, "at_seq": args.at_seq,
            "started": time.strftime("%Y-%m-%dT%H:%M:%S")}
    json.dump(meta, open(os.path.join(args.out, "meta.json"), "w"), indent=1)
    print(json.dumps(meta), flush=True)
    drive(args, design, task, branch, specs, meta, from_seq)


def end_all(agents: dict) -> dict:
    """Ends every agent at once (each reports its end to Arena)."""
    ends = {}
    threads = [threading.Thread(target=lambda a=a, x=x: ends.__setitem__(a, x.end()))
               for a, x in agents.items()]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    return ends


def drive(args, design, task, branch, specs, meta, from_seq):
    log = open_log(args.out)
    if args.fork_of:  # a new run recorded its team's starts; a fork's agents start again here
        for agent_id in specs:
            arena_api.register(branch, {"id": agent_id})
    agents = {a: Agent(branch, a, args.model, url, args.out, args.context_window)
              for a, (url, _, _) in specs.items()}
    first = {a: first_message(p, task) + history_note(h) for a, (_, p, h) in specs.items()
             if a in design.starts or h}
    engine = Engine(design, branch, agents, first, log, from_seq)
    stop = threading.Event()
    arena_api.follow(branch, engine.on_event, stop)
    time.sleep(1)
    outcome = engine.run(args.max_minutes * 60)
    meta.update(outcome=outcome, statuses=engine.statuses(), wakes=engine.wakes,
                ends=end_all(agents))
    stop.set()
    meta["finish"] = arena_api.finish(branch)
    meta["ended"] = time.strftime("%Y-%m-%dT%H:%M:%S")
    json.dump(meta, open(os.path.join(args.out, "meta.json"), "w"), indent=1)
    print(json.dumps({k: meta[k] for k in ("outcome", "statuses", "finish")}), flush=True)


if __name__ == "__main__":
    main()
