"""A deployed agent Arena can test (`kind: "http"`, protocol `arena-agent/1`, docs/agents.md).

    uv run python examples/agent_server.py [port]      # then connect http://127.0.0.1:8765 in Arena

Arena POSTs each run here; the agent works the task through the run's MCP endpoint with a real model
and answers when done. Claude API (``anthropic`` package) when ``ANTHROPIC_API_KEY`` is set, else
Claude Code (``claude -p``). Model: ``AGENT_MODEL`` (default ``claude-haiku-5-5``, or ``haiku`` for Claude Code).
"""

import json, os, subprocess, sys, tempfile
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import httpx

API = bool(os.environ.get("ANTHROPIC_API_KEY"))
MODEL = os.environ.get("AGENT_MODEL", "claude-haiku-5-5" if API else "haiku")


def mcp(run: dict, method: str, params: dict | None = None) -> dict:
    body = {"jsonrpc": "2.0", "id": 1, "method": method, "params": params or {}}
    r = httpx.post(run["mcp_url"], json=body, headers={"Authorization": f"Bearer {run['token']}"}, timeout=300)
    return r.json()["result"]


def claude_api(run: dict) -> dict:
    import anthropic
    tools = [{"name": t["name"], "description": t.get("description", ""), "input_schema": t["inputSchema"]}
             for t in mcp(run, "tools/list")["tools"]]
    client, messages, usage = anthropic.Anthropic(), [{"role": "user", "content": run["task"]}], [0, 0]
    for _ in range(40):
        r = client.messages.create(model=MODEL, max_tokens=16000, tools=tools, messages=messages)
        usage = [usage[0] + r.usage.input_tokens, usage[1] + r.usage.output_tokens]
        messages.append({"role": "assistant", "content": r.content})
        calls = [b for b in r.content if b.type == "tool_use"]
        if r.stop_reason != "tool_use" or not calls:
            break
        results = [mcp(run, "tools/call", {"name": c.name, "arguments": c.input}) for c in calls]
        messages.append({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": c.id, "content": res["content"][0]["text"], "is_error": res["isError"]}
            for c, res in zip(calls, results)]})
    return {"ok": True, "usage": {"input_tokens": usage[0], "output_tokens": usage[1]}}


def claude_code(run: dict) -> dict:
    server = {"type": "http", "url": run["mcp_url"], "headers": {"Authorization": f"Bearer {run['token']}"}}
    with tempfile.NamedTemporaryFile("w", suffix=".json") as cfg:   # the token stays off the command line
        json.dump({"mcpServers": {"world": server}}, cfg)
        cfg.flush()
        out = subprocess.run(["claude", "-p", "--model", MODEL, "--output-format", "json", "--strict-mcp-config",
                              "--mcp-config", cfg.name, "--tools", "", "--allowedTools", "mcp__world__*",
                              "--setting-sources", "", "--no-session-persistence", "--", run["task"]],
                             capture_output=True, text=True, timeout=1800)
    res = json.loads(out.stdout or "{}")
    u = res.get("usage", {})
    tokens_in = sum(u.get(k, 0) for k in ("input_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"))
    return {"ok": not res.get("is_error", True), "error": res.get("result") if res.get("is_error") else None,
            "usage": {"input_tokens": tokens_in, "output_tokens": u.get("output_tokens", 0)},
            "cost_usd": res.get("total_cost_usd")}


class Agent(BaseHTTPRequestHandler):
    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        if req.get("type") == "hello":
            answer = {"name": f"Example agent ({MODEL})"}
        else:
            print(f"run {req['run_id']}: working until {req['deadline']}", flush=True)
            answer = claude_api(req) if API else claude_code(req)
        body = json.dumps(answer).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(body)


port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
print(f"agent on http://127.0.0.1:{port} ({'Claude API' if API else 'Claude Code'}, {MODEL})", flush=True)
ThreadingHTTPServer(("127.0.0.1", port), Agent).serve_forever()
