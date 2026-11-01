"""Checking what callers pass: tool arguments against their parameters, and config values."""
from dataclasses import dataclass
from datetime import datetime

from . import errors


class Refusal(Exception):
    """A tool refused the call (bad argument, unknown id): answered as a tool-level `error`."""


REQUIRED = object()
TYPES = {"string": (str,), "integer": (int,), "number": (int, float), "boolean": (bool,), "array": (list,),
         "object": (dict,)}


@dataclass(frozen=True)
class Param:
    name: str
    type: str  # a JSON Schema type, a key of TYPES
    default: object = REQUIRED

    def check(self, value):
        ok = isinstance(value, TYPES[self.type]) and (self.type == "boolean" or not isinstance(value, bool))
        if not ok and not (value is None and self.default is None):
            raise Refusal(f"{self.name} must be of type {self.type}")


def input_schema(params) -> dict:
    """The MCP `inputSchema` of a tool with these params."""
    props = {p.name: {"type": p.type} for p in params}
    return {"type": "object", "properties": props, "required": [p.name for p in params if p.default is REQUIRED]}


def normalize(tool: str, params, args) -> dict:
    """`args` checked against `params`, with defaults filled in; Refusal when they don't fit."""
    if not isinstance(args, dict):
        raise Refusal("args must be an object")
    out = {}
    for p in params:
        if p.name in args:
            p.check(args[p.name])
            out[p.name] = args[p.name]
        elif p.default is REQUIRED:
            raise Refusal(f"{tool}: missing {p.name}")
        else:
            out[p.name] = p.default
    return out


def refused(message: str) -> dict:
    """A call_tool result for a refusal: the error and no keys touched."""
    return {"error": message, "reads": [], "writes": []}


def config_datetime(config: dict, key: str, default: str) -> datetime:
    """`config[key]` (or `default`) as an ISO datetime; `invalid` otherwise."""
    raw = config.get(key) or default
    try:
        return datetime.fromisoformat(raw)
    except (TypeError, ValueError):
        raise errors.invalid(f"{key} must be an ISO datetime, got {raw!r}") from None
