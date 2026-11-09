# Claude Code agents in an NVIDIA OpenShell sandbox

[OpenShell](https://docs.nvidia.com/openshell/latest/) (Apache 2.0) runs a program in a sandbox whose network,
files and processes follow a YAML policy, and logs every denial. With this folder first in arenad's `PATH`,
every Claude Code agent arenad starts runs in a fresh sandbox under a policy, and the sandbox's denials are
recorded on the run's timeline as world events (part `sandbox`, kind `denial`) with their reasons.

| File | What it is |
|---|---|
| `claude` | Stands in for `claude`: makes a sandbox, runs the real Claude Code in it, posts its denials, deletes it. |
| `denials.py` | Turns `openshell logs` lines into events and posts them to `POST /api/branches/{id}/sandbox-events`. |
| `factory-01.yaml` | The policy for Factory-01: the agent may reach only Arena's MCP endpoint and Anthropic's API. |
| `Dockerfile` | The sandbox image: OpenShell's Ubuntu base and this machine's Claude Code binary. |

## Set up (no root needed; Docker driver)

What was verified on 2026-11-08, with OpenShell v0.1.7 and Claude Code 2.1.319 on Ubuntu (kernel 6.17):

1. Download the release binaries `openshell-x86_64-unknown-linux-musl.tar.gz` and
   `openshell-gateway-x86_64-unknown-linux-gnu.tar.gz` from github.com/NVIDIA/OpenShell into `$OPENSHELL_HOME/bin`.
2. Gateway config (`$OPENSHELL_HOME/config/gateway.toml`): `[openshell] version = 2`; under
   `[openshell.gateway]` `bind_address = "127.0.0.1:17670"` and `compute_driver = "docker"`.
3. With `XDG_STATE_HOME=$OPENSHELL_HOME/state` and `XDG_CONFIG_HOME=$OPENSHELL_HOME/cliconf`:
   `openshell-gateway generate-certs --output-dir $XDG_STATE_HOME/openshell/tls --server-san host.openshell.internal`,
   start `openshell-gateway --config config/gateway.toml` (with `OPENSHELL_LOCAL_TLS_DIR` set to that folder),
   then `openshell gateway add https://127.0.0.1:17670 --local --name arena`.
4. Build the image (see `Dockerfile`) and tag it `openshell-arena-claude:latest`.
5. Start arenad with `PATH=<this folder>:$PATH OPENSHELL_HOME=...` (optionally `ARENA_OPENSHELL_POLICY`,
   `ARENA_OPENSHELL_IMAGE`).

## How it works, and its limits

- The sandbox can't reach its own loopback, and a policy can never allow loopback; it reaches the host as
  `host.openshell.internal`. The wrapper rewrites `127.0.0.1:` in Claude Code's arguments (the MCP URL) to that.
- Claude Code signs in with a copy of this user's `~/.claude/.credentials.json`, uploaded into the sandbox and
  deleted with it. OpenShell's built-in `claude-code` provider takes API keys only; a provider holding a
  `CLAUDE_CODE_OAUTH_TOKEN` would keep the real credential out of the sandbox (not tried).
- OpenShell inspects TLS (it sets `NODE_EXTRA_CA_CERTS` in the sandbox); Claude Code worked with it unchanged.
- Denials are read from `openshell logs <sandbox> --source sandbox` when the agent ends, before the sandbox is
  deleted (its logs go with it), so they land on the timeline just before the agent's end, at that clock.
  Lines look like `NET:OPEN [MED] DENIED /usr/local/bin/claude(0) -> mcp-proxy.anthropic.com:443
  [reason:transparent_tcp_policy_denied]`; one event per denied host, with the count and the reasons.
- Each agent start pays for a sandbox (about 3 s on this machine).
