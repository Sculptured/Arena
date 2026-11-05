"""Arena's HTTP API, as an external design uses it (docs/reference.md): start a run, register
agents, report what their clients saw, end them, finish, fork, and follow a branch's events."""
import json
import threading
import urllib.error
import urllib.request

BASE = "http://127.0.0.1:4510"


class ArenaError(RuntimeError):
    pass


def call(method: str, path: str, body=None, timeout: float = 600):
    """One request; the answer's JSON. An error status raises `ArenaError` with Arena's message."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method,
                                 headers={"Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            text = r.read().decode()
    except urllib.error.HTTPError as e:
        raise ArenaError(f"{method} {path}: {e.code} {e.read().decode()[:500]}") from None
    return json.loads(text) if text else None


def task(task_id: str) -> dict:
    """The task's ticket and the benchmark's instructions."""
    found = [t for t in call("GET", "/api/tasks") if t["id"] == task_id]
    if not found:
        raise ArenaError(f"the benchmark has no task {task_id}")
    return found[0]


def start_run(task_id: str, team: dict) -> dict:
    return call("POST", "/api/runs", {"task_id": task_id, "team": team, "runner": "external"})


def register(branch: str, agent: dict) -> dict:
    return call("POST", f"/api/branches/{branch}/agents", agent)


def report_usage(branch: str, agent: str, report: dict) -> None:
    call("POST", f"/api/branches/{branch}/agents/{agent}/usage", report, timeout=60)


def end_agent(branch: str, agent: str, done: dict) -> None:
    call("POST", f"/api/branches/{branch}/agents/{agent}/end", done)


def finish(branch: str) -> dict:
    return call("POST", f"/api/branches/{branch}/finish", {}, timeout=3600)


def fork(branch: str, at_seq: int, changes: dict) -> dict:
    body = {"at_seq": at_seq, "changes": changes, "restore": "exact"}
    return call("POST", f"/api/branches/{branch}/fork", body, timeout=3600)


def mcp_call(mcp_url: str, tool: str, args: dict) -> dict:
    """Calls `tool` through an agent's MCP endpoint, as that agent (so Arena records it as the
    agent's)."""
    body = {"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": tool, "arguments": args}}
    req = urllib.request.Request(mcp_url, data=json.dumps(body).encode(), method="POST",
                                 headers={"Content-Type": "application/json",
                                          "Accept": "application/json, text/event-stream"})
    with urllib.request.urlopen(req, timeout=120) as r:
        return json.loads(r.read().decode() or "{}")


def branch(branch_id: str) -> dict:
    return call("GET", f"/api/branches/{branch_id}")


def follow(branch_id: str, on_event, stop: threading.Event) -> threading.Thread:
    """Calls `on_event(event)` for each event of the branch's stream (Server-Sent Events), in a
    thread, until the stream ends or `stop` is set."""
    def run():
        req = urllib.request.Request(f"{BASE}/api/branches/{branch_id}/stream")
        with urllib.request.urlopen(req, timeout=None) as r:
            name, data = None, []
            for raw in r:
                if stop.is_set():
                    return
                line = raw.decode().rstrip("\n")
                if line.startswith("event:"):
                    name = line[6:].strip()
                elif line.startswith("data:"):
                    data.append(line[5:].strip())
                elif not line and data:
                    if name == "event":
                        on_event(json.loads("\n".join(data)))
                    name, data = None, []
    t = threading.Thread(target=run, daemon=True)
    t.start()
    return t
