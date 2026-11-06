"""Standard-library HTTP/MCP client; never calls a model or a model gateway."""
import json
import shutil
import time
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TEAMS = json.loads((ROOT / "benchmarks/pharma/data/teams.json").read_text())
DISCLOSURE = "scripted agents, illustrative usage"


def disk_guard():
    free = shutil.disk_usage("/").free
    if free < 12 * 1024**3:
        raise RuntimeError(f"Stopping: only {free / 1024**3:.2f} GiB free on /")


class API:
    def __init__(self, url):
        self.url = url.rstrip("/")

    def request(self, path, body=None):
        disk_guard()
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.url + path, data=data, headers={
            "Content-Type": "application/json", "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=1800) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            raise RuntimeError(f"{path}: HTTP {error.code}: {error.read().decode()}") from error


def deployment_team(deployment):
    from behaviour import NAMES
    key = "single" if deployment == "single" else "team"
    design = next(d for d in TEAMS["deployments"] if d["id"] == key)
    agents = [{"id": r["id"], "name": r["name"], "kind": "external",
               "model": "scripted-illustrative", "read_only": r["read_only"],
               "prompt": f"{r['prompt']} Demo: {DISCLOSURE}."} for r in design["roles"]]
    return {"id": f"demo-{deployment}", "name": f"Demo: {NAMES[deployment]}", "agents": agents}


class Calls:
    def __init__(self, api, started, deployment, pace, behaviour):
        self.api, self.branch = api, started["branch_id"]
        self.deployment, self.pace, self.behaviour = deployment, pace, behaviour
        self.counts, self.totals = {}, {}
        key = "single" if deployment == "single" else "team"
        design = next(d for d in TEAMS["deployments"] if d["id"] == key)
        self.roles = {r["id"]: r for r in design["roles"]}

    def actor(self, role):
        return "operator" if self.deployment == "single" else role

    def call(self, role, tool, args=None, refused=False):
        agent = self.actor(role)
        allowed = {t for group in self.roles[agent]["tools"] for t in TEAMS["tools"][group]}
        if tool not in allowed | {"message", "inbox", "handoff"}:
            raise ValueError(f"{agent} cannot use {tool}")
        time.sleep(self.pace + self.behaviour.delay)
        n = self.counts.get(agent, 0) + 1
        self.counts[agent] = n
        self.usage(agent, n)
        result = self.api.request(f"/mcp/{self.branch}/{agent}", {
            "jsonrpc": "2.0", "id": n, "method": "tools/call",
            "params": {"name": tool, "arguments": args or {}}})
        if "error" in result:
            raise RuntimeError(result["error"])
        out = result["result"]
        if bool(out.get("isError")) != refused:
            raise RuntimeError(f"{agent}/{tool}: expected refusal={refused}: {out}")
        text = out["content"][0]["text"]
        print(f"{self.branch} {agent}: {tool}" + (" [expected refusal]" if refused else ""), flush=True)
        return text if refused else json.loads(text)

    def usage(self, agent, n):
        prompt = 1600 + n * (240 if self.deployment == "single" else 145)
        completion = 95 + (n % 5) * 23
        self.api.request(f"/api/branches/{self.branch}/agents/{agent}/usage", {
            "message_id": f"demo-{self.branch}-{agent}-{n}", "model": "scripted-illustrative",
            "input_tokens": prompt, "output_tokens": completion, "context_window": 200000})
        old = self.totals.get(agent, (0, 0))
        self.totals[agent] = (old[0] + prompt, old[1] + completion)

    def message(self, sender, recipient, facts):
        if self.deployment != "single":
            self.call(sender, "message", {"to": recipient, "text": json.dumps(facts)})
            self.call(recipient, "inbox")

    def approve(self, role, action, payload):
        for _ in range(2):
            change = self.call(role, "propose_change", {
                "action": action, "payload": payload, "summary": f"Demo: {action}"})
            changes = self.call(role, "list_changes")["changes"]
            decided = next(c for c in changes if c["id"] == change["id"])
            if decided["decision"]["approved"]:
                return change["id"]
        raise RuntimeError(f"{action} was not approved: {decided}")

    def end(self, error=None):
        for agent in self.roles:
            prompt, completion = self.totals.get(agent, (0, 0))
            self.api.request(f"/api/branches/{self.branch}/agents/{agent}/end", {
                "ok": error is None, "error": error, "prompt_tokens": prompt,
                "completion_tokens": completion,
                "cost_usd": (prompt * 3 + completion * 15) / 1_000_000,
                "cost_source": "illustrative scripted usage; no model billed"})
        return self.api.request(f"/api/branches/{self.branch}/finish", {})
