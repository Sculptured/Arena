"""Errors a part reports to arenad as JSON-RPC errors."""


class PartError(Exception):
    """A JSON-RPC error with an arena-part/1 code."""

    def __init__(self, code: int, message: str):
        super().__init__(message)
        self.code = code
        self.message = message

    def __str__(self) -> str:
        return f"{self.message} ({self.code})"


def cant_restore(message: str) -> PartError:
    """-32001: can't do that restore."""
    return PartError(-32001, message)


def unknown(message: str) -> PartError:
    """-32002: unknown instance or save."""
    return PartError(-32002, message)


def fingerprint(message: str) -> PartError:
    """-32003: fingerprint mismatch."""
    return PartError(-32003, message)


def invalid(message: str) -> PartError:
    """-32602: invalid params."""
    return PartError(-32602, message)


def internal(message: str) -> PartError:
    """-32603: internal error."""
    return PartError(-32603, message)


def method_not_found(method: str) -> PartError:
    """-32601: method not found."""
    return PartError(-32601, f"method not found: {method}")
