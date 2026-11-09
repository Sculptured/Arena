# Bring your own agent

Shakedown can test an agent system that is already deployed, wherever it runs, against its simulated plant. The
agent stays where it is; Shakedown orchestrates the run, records every tool call it makes, scores the world with
the benchmark's checks and keeps the run. Two kinds of team member do this:

| Kind | Team member | What runs the agent |
|---|---|---|
| `http` | `{kind: "http", url, headers?}` | Your service, behind a URL |
| `bedrock` | `{kind: "bedrock", model_id, region}` | arenad's own tool-use loop on Amazon Bedrock's Converse API |

Either can be one member of a team next to Claude Code or API-mode agents, under the `chain` or `concurrent`
runner, in a single run or an experiment. In the web UI: **Connect your agent** in the team picker of New
experiment and Run once, then **Test connection** and **Use this agent**.

## HTTP agents (`arena-agent/1`)

**Run.** When the agent's turn starts, arenad sends one `POST` to its URL, with its headers:

```json
{
  "protocol": "arena-agent/1",
  "type": "run",
  "run_id": "0abe2f29…",
  "branch_id": "20ba9b19…",
  "agent": "planner",
  "task": "<the agent's prompt, the ticket, the instructions and any handoff notes>",
  "mcp_url": "http://127.0.0.1:4510/mcp/<branch>/<agent>",
  "token": "<bearer token for mcp_url>",
  "deadline": "2026-11-09T04:18:33Z"
}
```

The agent works through `mcp_url`, an MCP endpoint (streamable HTTP, JSON answers) with the world's tools and
the talk tools, sending `Authorization: Bearer <token>` on every call; a call without it gets 401. Every tool call
is recorded as for any other agent, refusals included.

**Done is the HTTP answer.** The agent answers the `POST` when it has finished: any 2xx means done, optionally
with `{"ok": true, "usage": {"input_tokens": n, "output_tokens": n}, "cost_usd": x}`, which the run records as
the agent's tokens and cost (labelled `agent_reported`). `{"ok": false, "error": "…"}` says it failed. There is
no `finish` tool: one request, one result, no extra tool in the agent's list, and an error status means what it
says. The cost is that the request stays open while the agent works (up to 15 minutes, the `deadline`), so a
gateway in front of the agent with a shorter timeout (AWS API Gateway cuts at 29 s) needs a direct URL.

**How a run ends.** Each case is the agent's `agent_end` event and the branch's reason:

| What happened | Branch | Reason recorded |
|---|---|---|
| 2xx | scored by the checks | |
| `{"ok": false}` | `failed` | `the agent says it failed: …` |
| No answer by the deadline | `failed` | `the agent at <host> did not answer within 900 s` |
| An error status | `infra_error` | `the agent at <host> answered HTTP 502: <start of its body>` |
| Not reachable | `infra_error` | `could not reach the agent at <host>: …` |

Time the branch spends paused doesn't count. Once arenad stops waiting, the token no longer opens the endpoint.

**Hello.** **Test connection** (`POST /api/agents/check`) sends `{"protocol": "arena-agent/1", "type": "hello"}`
with the same headers and waits up to 10 s; any 2xx is a handshake, and `{"name": "…"}` names the agent.

**Secrets.** Header values never reach a log, an API answer or a saved run. arenad keeps each value in memory the
moment it reads it, and everywhere else it stands as a reference, `{"secret": "<ref>"}`, which can be sent back in
its place. A reference lasts until arenad stops; a team that names one from before is refused with "connect the
agent again". Errors name the URL's host, never its path or query, but the URL itself is stored with the team:
put credentials in headers, not in the URL.

**Where the agent runs.** arenad listens on 127.0.0.1 only. An agent on another machine reaches `mcp_url` through
a tunnel you run; set `ARENA_AGENT_BASE` to the tunnel's base URL and `mcp_url` uses it.

**Example.** `sdk/python/examples/agent_server.py` is a complete HTTP agent in about 70 lines: it takes the run
and works the task with a real model, through the Claude API (`anthropic` package) when `ANTHROPIC_API_KEY` is
set, else through Claude Code (`claude -p` with the MCP endpoint and token in its MCP config).

```bash
(cd sdk/python && uv run python examples/agent_server.py 8765)   # then connect http://127.0.0.1:8765
```

**Verified (2026-11-08):** the example on Claude Code, model `haiku`, one run of the Factory-01 task
`supplier-delay` through a local arenad: it read the schedule, stock and suppliers, reserved the 24 free motors,
was refused the USD 2,000 purchase (`PO_LIMIT_EXCEEDED`), raised a flag for a human, and the run passed every
check, in 24 s, 43k tokens, USD 0.0048 as Claude Code reported it. The header value used was in no record. The
example's Claude API path has not been run (no key on that machine). The protocol is tested in
`core/crates/arenad/tests/http_agent.rs` with a protocol client that calls the MCP tools (no model, no decisions).

## Bedrock agents

`{kind: "bedrock", model_id, region}` becomes an API-mode agent on provider `bedrock:<region>` (stored as
`{kind: "open", provider: "bedrock:us-east-1", model: "<model_id>"}`): arenad's own tool-use loop, with every
model call recorded through its gateway, translated to the Converse API (`POST
https://bedrock-runtime.<region>.amazonaws.com/model/<model_id>/converse`) and signed with Signature Version 4.

Credentials come from the user's own AWS setup, read when a call is made and never stored:
`AWS_BEARER_TOKEN_BEDROCK` (a Bedrock API key), else `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (and
`AWS_SESSION_TOKEN`), else whatever the AWS CLI resolves (`aws configure export-credentials`, honouring
`AWS_PROFILE`: shared files, IAM Identity Center, `aws login`, assumed and instance roles), kept in memory until
five minutes before they expire. **Test connection** makes one Converse call of one output token.

**Untested live.** No Bedrock call has been made. The signing passes AWS's own test vector (`get-vanilla`), and
the request and answer translation, the signed request and the error path are tested against a local server
standing in for Bedrock (`core/crates/arenad/src/providers/bedrock*`). On the machine it was built on, the AWS
session had expired, and Test connection said so: "no usable AWS credentials: the aws CLI says: … Your session
has expired". Costs of Bedrock calls are not known to arenad and show as unknown. Claude's thinking blocks are
not sent back across turns (the loop doesn't ask for thinking).
