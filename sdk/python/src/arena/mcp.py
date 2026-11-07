"""A minimal client for an agent's MCP endpoint (the world's tools and the talk tools).

arenad's endpoint answers JSON-RPC over plain HTTP POST (MCP streamable HTTP, JSON answers), so a few
lines do: ``tools/list`` and ``tools/call``. Any MCP client works too: point it at ``mcp_url``.
"""

from __future__ import annotations

import itertools
import json
from dataclasses import dataclass, field
from typing import Any, Optional

import httpx

from .errors import ArenaError, ConnectionFailed


@dataclass
class Tool:
    name: str
    description: str
    input_schema: dict[str, Any]

    @classmethod
    def from_dict(cls, d: dict[str, Any]) -> "Tool":
        return cls(
            name=d["name"],
            description=d.get("description", ""),
            input_schema=d.get("inputSchema") or d.get("input_schema") or {"type": "object"},
        )

    def openai_schema(self) -> dict[str, Any]:
        """The tool as an OpenAI chat-completions ``tools`` entry."""
        return {
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.input_schema,
            },
        }

    def anthropic_schema(self) -> dict[str, Any]:
        """The tool as a Claude Messages API ``tools`` entry."""
        return {"name": self.name, "description": self.description, "input_schema": self.input_schema}


@dataclass
class ToolResult:
    """A tool call's outcome. A refused or failed call is ``is_error`` with arenad's or the part's
    message in ``text``; it is recorded like any other call."""

    text: str
    is_error: bool
    structured: Optional[dict[str, Any]] = None
    raw: dict[str, Any] = field(default_factory=dict, repr=False)

    @property
    def value(self) -> Any:
        """The result as data: the structured result, else the text parsed as JSON, else the text."""
        if self.structured is not None:
            return self.structured
        try:
            return json.loads(self.text)
        except (ValueError, TypeError):
            return self.text


class McpError(ArenaError):
    """The MCP endpoint answered a JSON-RPC error (unknown branch or agent, bad request)."""


class McpClient:
    def __init__(self, url: str, http: Optional[httpx.Client] = None, timeout: float = 600.0):
        self.url = url
        self._http = http or httpx.Client(timeout=timeout)
        self._ids = itertools.count(1)

    def _rpc(self, method: str, params: Optional[dict[str, Any]] = None) -> Any:
        body = {"jsonrpc": "2.0", "id": next(self._ids), "method": method, "params": params or {}}
        try:
            r = self._http.post(
                self.url,
                json=body,
                headers={"accept": "application/json, text/event-stream"},
            )
        except httpx.TransportError as e:
            raise ConnectionFailed(f"MCP endpoint {self.url}: {e}") from e
        try:
            msg = r.json()
        except ValueError as e:
            raise McpError(f"MCP endpoint answered {r.status_code} without JSON") from e
        if "error" in msg:
            raise McpError(msg["error"].get("message", str(msg["error"])))
        return msg.get("result")

    def initialize(self) -> dict[str, Any]:
        return self._rpc(
            "initialize",
            {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "arena-sdk"}},
        )

    def list_tools(self) -> list[Tool]:
        return [Tool.from_dict(t) for t in self._rpc("tools/list")["tools"]]

    def call_tool(self, name: str, arguments: Optional[dict[str, Any]] = None) -> ToolResult:
        res = self._rpc("tools/call", {"name": name, "arguments": arguments or {}})
        text = "".join(c.get("text", "") for c in res.get("content", []) if c.get("type") == "text")
        return ToolResult(
            text=text,
            is_error=bool(res.get("isError")),
            structured=res.get("structuredContent"),
            raw=res,
        )
