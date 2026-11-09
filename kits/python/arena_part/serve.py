"""Line protocol loop, method dispatch and id validation. Serial: one request at a time."""
import json
import re
import sys

from . import errors
from .errors import PartError

_ID = re.compile(r"[a-z0-9_]{1,63}")
_ID_MSG = "instance and save ids must match [a-z0-9_]{1,63}"


class Part:
    """An integration: one program that holds many instances and saves of one kind of system.
    Methods take and return plain JSON values (dicts, lists). Override what you need."""

    def describe(self):
        raise NotImplementedError("describe")

    def start(self, instance, config, seed, clock_ms, from_=None):
        raise NotImplementedError("start")

    def call_tool(self, instance, name, args, clock_ms):
        raise NotImplementedError("call_tool")

    def advance(self, instance, clock_ms):
        return {"changed": False, "events": []}

    def save(self, instance, save, dir):
        raise NotImplementedError("save")

    def inspect(self, target, keys):
        raise NotImplementedError("inspect")

    def diff(self, a, b):
        raise NotImplementedError("diff")

    def drop_save(self, save):
        raise NotImplementedError("drop_save")

    def stop(self, instance):
        raise NotImplementedError("stop")

    def usage(self):
        raise NotImplementedError("usage")

    def gc(self, keep):
        return {"dropped": 0, "bytes": 0}

    def connect(self, instance):
        raise errors.method_not_found("connect")


def valid_id(v) -> bool:
    """True for an instance or save id (`[a-z0-9_]{1,63}`)."""
    return isinstance(v, str) and _ID.fullmatch(v) is not None


def _valid_keep(v) -> bool:
    """A `gc` keep entry: a save id, or an id prefix followed by `*` (`base_*`)."""
    if isinstance(v, str) and v.endswith("*"):
        return v == "*" or valid_id(v[:-1])
    return valid_id(v)


def kept(keep, save_id) -> bool:
    """True when the `gc` keep entries keep `save_id` (an entry ending in `*` is a prefix)."""
    return any(save_id.startswith(k[:-1]) if k.endswith("*") else k == save_id for k in keep)


def ids_ok(params) -> bool:
    """Every `instance`, `save` (also inside target, a, b, from) must be a valid id, and every
    `keep` entry an id or an id prefix followed by `*`."""
    if not isinstance(params, dict):
        return True
    for k, v in params.items():
        if k in ("instance", "save"):
            ok = valid_id(v)
        elif k == "keep":
            ok = not isinstance(v, list) or all(_valid_keep(s) for s in v)
        elif k in ("target", "a", "b", "from"):
            ok = ids_ok(v)
        else:
            ok = True
        if not ok:
            return False
    return True


def _need(params, *names):
    if not isinstance(params, dict):
        raise errors.invalid("invalid params: expected an object")
    missing = [n for n in names if n not in params]
    if missing:
        raise errors.invalid(f"invalid params: missing {', '.join(missing)}")
    return [params[n] for n in names]


def _start(part, params):
    instance, config, seed, clock_ms = _need(params, "instance", "config", "seed", "clock_ms")
    part.start(instance, config, seed, clock_ms, params.get("from"))
    return {}


def _call_tool(part, params):
    out = dict(part.call_tool(*_need(params, "instance", "name", "args", "clock_ms")))
    out.setdefault("reads", [])
    out.setdefault("writes", [])
    return out


def _unit(method, *names):
    """A method that takes `names` from the params and answers `{}`."""

    def run(part, params):
        getattr(part, method)(*_need(params, *names))
        return {}

    return run


def _plain(method, *names):
    """A method that takes `names` from the params and answers with the part's result."""
    return lambda part, params: getattr(part, method)(*_need(params, *names))


_METHODS = {
    "describe": lambda part, params: part.describe(),
    "start": _start,
    "call_tool": _call_tool,
    "advance": _plain("advance", "instance", "clock_ms"),
    "save": _plain("save", "instance", "save", "dir"),
    "inspect": _plain("inspect", "target", "keys"),
    "diff": _plain("diff", "a", "b"),
    "drop_save": _unit("drop_save", "save"),
    "stop": _unit("stop", "instance"),
    "usage": lambda part, params: part.usage(),
    "gc": _plain("gc", "keep"),
    "connect": _plain("connect", "instance"),
}


def dispatch(part, method, params):
    handler = _METHODS.get(method)
    if handler is None:
        raise errors.method_not_found(method)
    return handler(part, params)


def _response(id_, out=None, err=None):
    if err is not None:
        return {"jsonrpc": "2.0", "id": id_, "error": {"code": err.code, "message": err.message}}
    return {"jsonrpc": "2.0", "id": id_, "result": out}


def _handle(part, req, dispatch_fn=dispatch):
    """Returns (id or None, response dict or None when no reply is due)."""
    if not isinstance(req, dict):
        return _response(None, err=PartError(-32600, "invalid request"))
    has_id = "id" in req
    id_ = req.get("id")
    method = req.get("method")
    params = req.get("params")
    if params is None:
        params = {}
    try:
        if not isinstance(method, str) or not method:
            raise PartError(-32600, "invalid request")
        if not ids_ok(params):
            raise errors.invalid(_ID_MSG)
        resp = _response(id_, dispatch_fn(part, method, params))
    except PartError as e:
        resp = _response(id_, err=e)
    except Exception as e:  # a raising handler is answered, not fatal
        print(f"arena-part: handler failed: {e!r}", file=sys.stderr)
        resp = _response(id_, err=errors.internal(f"internal error: {e}"))
    return resp if has_id else None


def serve(part, stdin=None, stdout=None, dispatch_fn=dispatch):
    """Serve a part until the input closes: read a request line, answer it, repeat. `dispatch_fn`
    maps a method to its handler (a benchmark's: `benchmark.dispatch`)."""
    stdin = stdin if stdin is not None else sys.stdin
    stdout = stdout if stdout is not None else sys.stdout
    for line in stdin:
        text = line.strip()
        if not text:
            continue
        try:
            req = json.loads(text)
        except ValueError as e:
            resp = _response(None, err=PartError(-32700, f"parse error: {e}"))
        else:
            resp = _handle(part, req, dispatch_fn)
        if resp is not None:
            try:
                data = json.dumps(resp, separators=(",", ":"))
            except (TypeError, ValueError) as e:  # a result that isn't JSON: answer, don't die
                print(f"arena-part: unserializable result: {e}", file=sys.stderr)
                err = errors.internal(f"internal error: {e}")
                data = json.dumps(_response(resp.get("id"), err=err), separators=(",", ":"))
            stdout.write(data + "\n")
            stdout.flush()
