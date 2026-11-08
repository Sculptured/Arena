"""The disruption a task's setup may schedule on the OT network (arena_part.disruptions): a remote-access session
that appears on the IT/OT gateway, into a zone, from a user the access list doesn't have."""
from arena_part.disruptions import Kind
from arena_part.params import Refusal

from . import network

FIELDS = ("session", "user", "source", "target")


def _check(config, do) -> None:
    if network.zone_meta(config, do["target"]) is None:
        raise Refusal(f"no zone {do['target']}")


def _session(config, state, do) -> list:
    session = {"id": do["session"], "user": do["user"], "source": do["source"], "target": do["target"],
               "via": config["gateway"]["id"], "activity": do.get("activity") or "connected"}
    return [f"session/{network.add_session(config, state, session)}"]


KINDS = {"remote_session": Kind({f: "string" for f in FIELDS}, _session, _check)}
