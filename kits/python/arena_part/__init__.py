"""Kit for writing `arena-part/1` integrations. See docs/protocol.md."""
from . import folder
from .clock import FixedStep
from .errors import PartError, cant_restore, fingerprint, unknown
from .serve import Part, kept, serve

PROTOCOL = 1

__all__ = [
    "PROTOCOL", "Part", "PartError", "FixedStep", "serve", "folder",
    "cant_restore", "unknown", "fingerprint", "kept",
]
