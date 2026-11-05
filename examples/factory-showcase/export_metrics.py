"""Writes the showcase's metrics.json (the shape arena-video's video/README.md documents) from
Arena's HTTP API only: each branch's events, metrics and checks (`GET /api/branches/{id}`) and,
for each design's repeats, Arena's compare (`GET /api/compare`).

    python export_metrics.py --design A=<branch>[,<branch>...] --design B=... --design C=... \\
        [--fork <branch>] [--out metrics.json]

The first branch of each design is the one the charts draw; every branch is in `runs`, for the
spread across repeats.
"""
import argparse
import json
import statistics
import sys
import urllib.request
from datetime import datetime

BASE = "http://127.0.0.1:4510"
LABELS = {"A": "A, hierarchical", "B": "B, event-driven", "C": "C, single agent"}
MES_DOCTYPES = {"Work Order", "Job Card", "Workstation", "Workstation Type", "Operation", "BOM",
                "Plant Floor"}
SYSTEMS = {"crm_": "CRM", "plant_state": "Plant", "machine_telemetry": "Plant",
           "vibration_spectrum": "Plant", "recent_events": "Plant", "wait_for_event": "Plant",
           "dispatch_technician": "Plant", "set_line_status": "Plant", "move_order": "Plant",
           "propose_change": "Approvals", "list_changes": "Approvals", "decide_change": "Approvals",
           "handoff": "Arena talk", "message": "Arena talk", "inbox": "Arena talk"}
ERP_TOOLS = {"get_doc", "list_docs", "insert_doc", "update_doc", "submit_doc"}


def get(path: str):
    with urllib.request.urlopen(f"{BASE}{path}", timeout=600) as r:
        return json.loads(r.read())


def when(text: str) -> datetime:
    return datetime.fromisoformat(text.replace("Z", "+00:00"))


def system_of(name: str, args: dict) -> str:
    if name in ERP_TOOLS:
        doctype = args.get("doctype") or (args.get("doc") or {}).get("doctype") or ""
        return "MES" if doctype in MES_DOCTYPES else "ERP"
    for prefix, system in SYSTEMS.items():
        if name == prefix or (prefix.endswith("_") and name.startswith(prefix)):
            return system
    return "Suppliers, carriers, customers"


def ok_call(e: dict, name: str) -> bool:
    p = e["payload"]
    return e["kind"] == "tool_call" and p.get("name") == name and not p.get("error")


def doc_written(e: dict, doctype: str) -> bool:
    p = e["payload"]
    if e["kind"] != "tool_call" or p.get("error") or p.get("name") not in ("insert_doc", "submit_doc", "update_doc"):
        return False
    args = p.get("args") or {}
    return doctype in (args.get("doctype"), (args.get("doc") or {}).get("doctype"))


def proposed(e: dict, action: str) -> bool:
    p = e["payload"]
    return ok_call(e, "propose_change") and (p.get("args") or {}).get("action") == action


MILESTONES = [
    ("first tool call", lambda e: e["kind"] == "tool_call"),
    ("diagnosis proposed", lambda e: proposed(e, "maintenance_order")),
    ("technician dispatched", lambda e: ok_call(e, "dispatch_technician")),
    ("purchase order awarded", lambda e: ok_call(e, "award_order")),
    ("purchase order in ERP", lambda e: doc_written(e, "Purchase Order")),
    ("freight booked", lambda e: ok_call(e, "book_freight")),
    ("repair scheduled", lambda e: doc_written(e, "ToDo")),
    ("first customer notified", lambda e: ok_call(e, "notify_customer")),
]
LAST = [("last customer notified", "notify_customer"), ("last CRM update", "crm_update_opportunity")]


def milestones(b: dict, own: list) -> list:
    t0 = when(b["created_at"])
    out = []
    for name, test in MILESTONES:
        hit = next((e for e in own if test(e)), None)
        out.append({"name": name, "t_s": hit and round((when(hit["at"]) - t0).total_seconds(), 1)})
    for name, tool in LAST:
        hits = [e for e in own if ok_call(e, tool)]
        out.append({"name": name, "t_s": hits and round((when(hits[-1]["at"]) - t0).total_seconds(), 1) or None})
    end = b.get("ended_at")
    out.append({"name": "finished", "t_s": end and round((when(end) - t0).total_seconds(), 1)})
    return out


def context(b: dict, own: list) -> tuple:
    """Per agent, each reported model call's context (`usage.prompt_tokens`) and compactions."""
    t0 = when(b["created_at"])
    calls, compactions = {}, {}
    for e in own:
        if e["kind"] != "model_call":
            continue
        t = round((when(e["at"]) - t0).total_seconds(), 1)
        p = e["payload"]
        if p.get("compaction") is not None:
            compactions.setdefault(e["agent"], []).append({"t_s": t, **p["compaction"]})
        elif "prompt_tokens" in (p.get("usage") or {}):
            calls.setdefault(e["agent"], []).append({"t_s": t, "prompt_tokens": p["usage"]["prompt_tokens"]})
    return calls, compactions


def messages(own: list) -> tuple:
    pairs, topics = {}, {}
    for e in own:
        if e["kind"] not in ("message", "handoff"):
            continue
        p = e["payload"]
        to = p.get("to") or "*"
        pairs[(e["agent"], to)] = pairs.get((e["agent"], to), 0) + 1
        text = (p.get("text") or p.get("note") or "").strip()
        if text.startswith("#"):
            topic = text.split()[0][1:]
            topics[topic] = topics.get(topic, 0) + 1
    out = [{"from": f, "to": t, "count": n} for (f, t), n in sorted(pairs.items())]
    return out, topics


def tool_calls(own: list) -> tuple:
    by_system, errors = {}, 0
    for e in own:
        if e["kind"] == "tool_call":
            p = e["payload"]
            s = system_of(p.get("name", ""), p.get("args") or {})
            by_system[s] = by_system.get(s, 0) + 1
            errors += bool(p.get("error"))
    return by_system, errors


def checks_of(own: list) -> dict:
    check = next((e["payload"] for e in reversed(own) if e["kind"] == "check"), None)
    named = (check or {}).get("checks") or []
    return {"passed": sum(c["passed"] for c in named), "total": len(named),
            "failed": [c["name"] for c in named if not c["passed"]],
            "detail": {c["name"]: c["detail"] for c in named if not c["passed"]}}


def summary(branch_id: str) -> dict:
    """One branch, all from Arena: its checks, times, tokens, context and talk."""
    b = get(f"/api/branches/{branch_id}")
    own = [e for e in b["events"] if e["seq"] >= (b.get("start_seq") or 0)]
    m = b["metrics"]
    calls, compactions = context(b, own)
    msgs, topics = messages(own)
    by_system, tool_errors = tool_calls(own)
    peaks = {a: max(c["prompt_tokens"] for c in xs) for a, xs in calls.items()}
    return {"branch": branch_id, "run": b["run_id"], "parent": b.get("parent_id"),
            "status": b["status"], "checks": checks_of(own), "milestones": milestones(b, own),
            "measured_s": m["measured_s"], "estimated_s": m["estimated_s"],
            "overhead_s": m["overhead_s"], "overhead_ms": m["overhead_ms"],
            "tokens": m["tokens"], "cost": m["cost"], "agents": m["agents"],
            "context": calls, "compactions": compactions, "peak_context": peaks,
            "messages": msgs, "topics": topics, "tool_calls": by_system,
            "tool_errors": tool_errors, "model_calls": sum(len(x) for x in calls.values())}


def spread(runs: list) -> dict:
    """Mean, min and max across a design's repeats, for the numbers that are always there."""
    def stat(xs):
        xs = [x for x in xs if x is not None]
        if not xs:
            return None
        return {"mean": round(statistics.mean(xs), 1), "min": min(xs), "max": max(xs), "n": len(xs)}
    return {"checks_passed": stat([r["checks"]["passed"] for r in runs]),
            "measured_s": stat([r["measured_s"] for r in runs]),
            "estimated_s": stat([r["estimated_s"] for r in runs]),
            "tokens": stat([r["tokens"] for r in runs]),
            "peak_context": stat([max(r["peak_context"].values(), default=None) for r in runs]),
            "compactions": stat([sum(len(x) for x in r["compactions"].values()) for r in runs])}


def compared(runs: list) -> list:
    """Arena's compare of the first repeat with each other one: where the record diverged, and
    which rows of the end state differ."""
    out = []
    for other in runs[1:]:
        c = get(f"/api/compare?a={runs[0]['branch']}&b={other['branch']}")
        out.append({"a": runs[0]["branch"], "b": other["branch"],
                    "first_divergent_seq": c.get("first_divergent_seq"),
                    "differing_rows": len(c.get("differing_rows") or []),
                    "not_compared": c.get("not_compared")})
    return out


def design_entry(key: str, branches: list, compare: bool) -> dict:
    runs = [summary(b) for b in branches]
    first = runs[0]
    return {"label": LABELS[key], "checks": {k: first["checks"][k] for k in ("passed", "total")},
            "milestones": first["milestones"], "agents": first["agents"],
            "context": first["context"], "compactions": first["compactions"],
            "messages": first["messages"], "topics": first["topics"],
            "tool_calls": first["tool_calls"],
            "time": {"measured_s": first["measured_s"], "estimated_s": first["estimated_s"]},
            "runs": runs, "spread": spread(runs), "compare": compared(runs) if compare else []}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--design", action="append", required=True, help="KEY=branch[,branch...]")
    ap.add_argument("--fork", action="append", default=[], help="a fork's branch id")
    ap.add_argument("--context-limit", type=int, default=200000)
    ap.add_argument("--no-compare", action="store_true")
    ap.add_argument("--out", default="metrics.json")
    args = ap.parse_args()
    out = {"sample": False, "context_limit": args.context_limit, "source": BASE, "designs": {}}
    for d in args.design:
        key, ids = d.split("=", 1)
        out["designs"][key] = design_entry(key, ids.split(","), not args.no_compare)
    out["forks"] = [summary(f) for f in args.fork]
    json.dump(out, open(args.out, "w"), indent=1)
    print(f"wrote {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
