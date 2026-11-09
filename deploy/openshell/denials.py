"""Reads `openshell logs <sandbox> --source sandbox` on stdin and posts its policy denials to arenad as world
events of the branch and agent the MCP URL (argv[1]) names. One event per denied destination, with how many
times and why. Prints what it posted to stderr; posts nothing when the log has no denial."""
import json
import re
import sys
import urllib.request
from collections import OrderedDict

# e.g. "NET:OPEN [MED] DENIED /usr/local/bin/claude(0) -> example.com:443 [reason:transparent_tcp_policy_denied]"
DENIED = re.compile(r"(?P<what>[A-Z]+:[A-Z]+) \[\w+\] DENIED (?:(?P<proc>\S+?)(?:\(\d+\))? -> )?"
                    r"(?P<dest>[^\s\[]+)(?: \[reason:(?P<reason>[^\]]+)\])?")


def denials(lines):
    seen = OrderedDict()
    for line in lines:
        m = DENIED.search(line)
        if not m:
            continue
        dest = m["dest"]
        host = dest.rsplit(":", 1)[0] if ":" in dest else dest
        d = seen.setdefault(host, {"host": host, "dest": dest, "process": None, "reasons": [], "count": 0})
        d["count"] += 1
        d["process"] = d["process"] or m["proc"]
        if ":" in dest:
            d["dest"] = dest
        if m["reason"] and m["reason"] not in d["reasons"]:
            d["reasons"].append(m["reason"])
    return list(seen.values())


def event(d) -> dict:
    who = (d["process"] or "a process").rsplit("/", 1)[-1]
    why = ", ".join(d["reasons"]) or "no policy allows it"
    return {"kind": "denial", "text": f"OpenShell denied {who} → {d['dest']} ({why}), {d['count']}×",
            "data": {"source": "openshell", **d}}


def main():
    m = re.match(r"(http://[^/]+)/mcp/([a-z0-9_]+)/([A-Za-z0-9_-]+)", sys.argv[1])
    found = denials(sys.stdin)
    if not m or not found:
        return
    base, branch, agent = m.groups()
    body = json.dumps({"agent": agent, "events": [event(d) for d in found]}).encode()
    req = urllib.request.Request(f"{base}/api/branches/{branch}/sandbox-events", body,
                                 {"content-type": "application/json"})
    urllib.request.urlopen(req, timeout=10).read()
    print(f"openshell: posted {len(found)} denial event(s) to branch {branch}", file=sys.stderr)


if __name__ == "__main__":
    main()
