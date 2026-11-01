# `arena-part/1`

The protocol between arenad and an integration (or benchmark) program, in any language. Kits: Rust
(`kits/rust`, crate `arena-part`) and Python (`kits/python`). Wire types for Rust: `arena_part::proto`.

## Transport

JSON-RPC 2.0, one message per line, over the program's stdin and stdout. arenad starts the program;
stderr is its log. The program exits when stdin closes. Requests may be in flight at the same time and
answered in any order; match them by `id`. Each response is one whole line.

## Manifest

`arena-package.toml` in each integration or benchmark folder:

```toml
kind = "integration"         # or "benchmark"
name = "postgres"            # [a-z0-9-]{1,40}, unique among installed packages
version = "0.1.0"
protocol = 1
run = { command = ["${ARENA_BIN_DIR}/arena-postgres"] }    # or: run = { image = "..." }
needs = { gpu_vram_gb = 0, services = [] }    # e.g. services = ["host.docker.internal:5544"]
config_schema = { ... }      # JSON Schema of the part's config in a world

[[example]]                  # tool calls the conformance check runs
tool = "get"
args = {}
```

`command` runs as a local program, for integrations installed from a folder you trust, and may contain
`${ARENA_BIN_DIR}`. `image` runs boxed in a container: no network except the services it declares, no
environment from arenad, disk only under the save storage given to it.

## Ids and keys

- Instance and save ids are chosen by arenad and match `^[a-z0-9_]{1,63}$`. The kits answer `-32602` to any
  other id.
- Package names match `^[a-z0-9-]{1,40}$`.
- Keys name a piece of a part's state, `<kind>/<id>` (for Postgres, `orders/#W1`). The part decides what
  its keys are.

## Calls from arenad to the part

| Method | Params | Result |
|---|---|---|
| `describe` | `{}` | `{name, version, protocol, restore: {exact: "save"\|"replay"\|"none", approximate: bool}, uses_clock: bool, fingerprint: {...}, tools: [ToolSchema]}` |
| `start` | `{instance, config, seed, clock_ms, from?: {save, mode: "exact"\|"approximate"}}` | `{}` |
| `call_tool` | `{instance, name, args, clock_ms}` | `{result?, error?, reads: [key], writes: [key]}` |
| `advance` | `{instance, clock_ms}` | `{changed: bool, events: [{kind, text, data}]}` |
| `save` | `{instance, save, dir}` | `{bytes, exact: bool}` |
| `inspect` | `{target: {instance}\|{save}, keys: [key]}` | `{docs: {key: doc\|null}}` |
| `diff` | `{a: target, b: target}` | `{differing: [key]}` |
| `drop_save` | `{save}` | `{}` |
| `stop` | `{instance}` | `{}` |
| `usage` | `{}` | `{bytes}` |
| `gc` (optional) | `{keep: [save]}` | `{dropped, bytes}` |
| `connect` (optional) | `{instance}` | connection info, e.g. `{url}` for `postgres` |

- `ToolSchema` is the MCP shape (`name`, `description`, `inputSchema`) plus `effect: "read"|"write"`. Tool
  names must be unique across a world; a world that has a clash is refused.
- `dir` is a folder arenad creates for that save under `ARENA_SAVES/<part>/<save>/`. A part that keeps
  saves natively (Postgres `snap_` databases) ignores it and reports `bytes` itself.
- **Save folders by convention:** a part finds any save's folder at `$ARENA_SAVES/<package name>/<save>/`.
  arenad passes env `ARENA_SAVES` to the program, and `save.dir` is that same path. So a part that has
  been restarted can still start from, inspect or diff an older save that is not in its memory; it answers
  `-32002` only when the folder is missing.
- `advance` moves the part's clock to `clock_ms`. Parts with `uses_clock: false` never get it. Events the
  part reports (an alarm, a shipment arriving) are recorded as world events.
- `save` reports `exact: false` when the save may not capture everything (an approximate save).
- `inspect` with an empty `keys` list returns every key the part has for that target.
- `usage` counts only what the part keeps outside `ARENA_SAVES` (databases, files elsewhere). arenad
  measures the save folders under `ARENA_SAVES` itself, so a part whose saves all live there reports 0.
- `drop_save` of a save the part doesn't have answers `-32002` or succeeds; it is never another error.
  arenad calls it without checking first and treats `-32002` as "already gone", which counts as success.
- `gc` drops the saves not in `keep` (and instances that are not running). A `keep` entry is a save id,
  or an id prefix followed by `*`: `base_*` keeps every save whose id starts with `base_`. After a
  `gc`, arenad removes the folders under `ARENA_SAVES/<part>/` whose names `keep` does not keep.

## Benchmark calls

A benchmark program speaks the same transport.

| Method | Params | Result |
|---|---|---|
| `describe` | `{}` | `{name, version, protocol, world: [{part, integration, config}], tasks: [{id, reason, ticket}], instructions, tools: [ToolSchema]}` |
| `seed` | `{connections: {part: info}}` | `{}` (loads the starting data into fresh instances) |
| `call_tool` | `{name, args, connections}` | `{result?, error?, part, reads, writes}` |
| `gold_actions` | `{task}` | `{actions: [{name, args}]}` |

A benchmark's tools reach parts only through `connect` info. Only parts with exact saves may be connected
(replay re-sends a part's own inputs, which a benchmark's direct writes are not).

## Errors

JSON-RPC errors with a plain message. They are for the protocol: a call that can't be done. A tool the
agent called wrongly or that the part refuses for its own reasons (no such order, out of stock, a bad
quantity) is a tool-level answer, `{error, reads, writes}` from `call_tool`, which the agent sees and can
act on. `-32602` is only for protocol-level bad params (a bad id, a missing field, a malformed target).

| Code | Meaning |
|---|---|
| `-32001` | can't do that restore |
| `-32002` | unknown instance or save |
| `-32003` | fingerprint mismatch |
| `-32601` | method not found |
| `-32602` | invalid params (including a bad id) |

## Clock contract

A part's state after `advance` to t must not depend on how the advances were split: the part steps
internally in fixed increments up to t. Advancing to 2500 in one call or in five calls gives the same state.

## Determinism contract for `exact: "replay"`

Given the same `seed`, the same `fingerprint`, and the same ordered inputs (`call_tool` at `clock_ms`,
`advance` to `clock_ms`), the part reaches the same state and gives the same results. arenad replays by
`start` with the original seed at clock 0, then for each recorded input `advance` to its `clock_ms` and
repeat the call. A result that differs from the recorded one stops the replay with "replay diverged". A
replay is refused when the `fingerprint` differs from the one recorded at the run's start. The conformance
check (`arena-part-check`) tests this.

So anything that changes how the part behaves must be in its `fingerprint` (its code, data files, the
version of a program it drives) or checked at `start`: a `config` that points at other code or data than
`describe` hashed is refused with `-32003` there. arenad also refuses an exact rerun when a replayed part's
`{integration, config}` in the benchmark's world differs from the one the run recorded.

## Worked example

A `counter` part (`kits/rust/examples/counter.rs`). `>` is arenad to the part, `<` the reply.

```
> {"jsonrpc":"2.0","id":1,"method":"describe","params":{}}
< {"jsonrpc":"2.0","id":1,"result":{"name":"counter","version":"0.1.0","protocol":1,"restore":{"exact":"save","approximate":false},"uses_clock":true,"fingerprint":{"example":"counter"},"tools":[{"name":"add","description":"Add n to the counter.","inputSchema":{"type":"object","properties":{"n":{"type":"integer"}}},"effect":"write"},{"name":"get","description":"Read the counter.","inputSchema":{"type":"object","properties":{}},"effect":"read"}]}}
> {"jsonrpc":"2.0","id":2,"method":"start","params":{"instance":"i1","config":{},"seed":1,"clock_ms":0}}
< {"jsonrpc":"2.0","id":2,"result":{}}
> {"jsonrpc":"2.0","id":3,"method":"call_tool","params":{"instance":"i1","name":"add","args":{"n":5},"clock_ms":0}}
< {"jsonrpc":"2.0","id":3,"result":{"result":{"value":5},"reads":[],"writes":["counter/value"]}}
> {"jsonrpc":"2.0","id":4,"method":"save","params":{"instance":"i1","save":"s1","dir":"/saves/counter/s1"}}
< {"jsonrpc":"2.0","id":4,"result":{"bytes":20,"exact":true}}
> {"jsonrpc":"2.0","id":5,"method":"start","params":{"instance":"Bad Id","config":{},"seed":1,"clock_ms":0}}
< {"jsonrpc":"2.0","id":5,"error":{"code":-32602,"message":"instance and save ids must match [a-z0-9_]{1,63}"}}
```
