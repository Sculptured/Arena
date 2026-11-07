"""Server-Sent Events parsing (arenad's ``/api/branches/{id}/stream``)."""

from __future__ import annotations

import json
from typing import Any, AsyncIterator, Iterable, Iterator


def parse(lines: Iterable[str]) -> Iterator[tuple[str, Any]]:
    """``(event name, data parsed as JSON)`` for each event in ``lines``; comments (keep-alives)
    and events without data are skipped."""
    name, data = "message", []
    for line in lines:
        line = line.rstrip("\r\n")
        if not line:
            if data:
                yield name, _json("\n".join(data))
            name, data = "message", []
        elif line.startswith(":"):
            continue
        else:
            key, _, value = line.partition(":")
            value = value[1:] if value.startswith(" ") else value
            if key == "event":
                name = value
            elif key == "data":
                data.append(value)
    if data:
        yield name, _json("\n".join(data))


async def aparse(lines: AsyncIterator[str]) -> AsyncIterator[tuple[str, Any]]:
    name, data = "message", []
    async for line in lines:
        line = line.rstrip("\r\n")
        if not line:
            if data:
                yield name, _json("\n".join(data))
            name, data = "message", []
        elif line.startswith(":"):
            continue
        else:
            key, _, value = line.partition(":")
            value = value[1:] if value.startswith(" ") else value
            if key == "event":
                name = value
            elif key == "data":
                data.append(value)
    if data:
        yield name, _json("\n".join(data))


def _json(s: str) -> Any:
    try:
        return json.loads(s)
    except ValueError:
        return s
