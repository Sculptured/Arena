"""The network as the config describes it (zones, conduits, the gateway, the access list, the sessions open at the
start and the incident playbook) and as the state changes it: which zones are isolated, which sessions are
revoked, and a numbered log of what was done, so the order of a response can be read from the end state."""
from arena_part import errors

BASE = "base"
REQUIRED = ("zones", "conduits", "gateway", "playbook")


def check_config(config) -> None:
    missing = [k for k in REQUIRED if k not in config]
    if missing:
        raise errors.invalid(f"the network's config needs {', '.join(missing)}")
    ids = [z.get("id") for z in config["zones"]]
    if not ids or not all(isinstance(i, str) for i in ids) or len(set(ids)) != len(ids):
        raise errors.invalid("zones must be a non-empty list with unique text ids")
    for c in config["conduits"]:
        if c.get("from") not in ids or c.get("to") not in ids:
            raise errors.invalid(f"conduit {c.get('id')} joins a zone the config doesn't have")


def zone_meta(config, zid: str):
    return next((z for z in config["zones"] if z["id"] == zid), None)


def new_state(config) -> dict:
    sessions = {s["id"]: {**s, "status": "active", "approved": _on_list(config, s)} for s in config.get("sessions", [])}
    return {"scenario": BASE, "zones": {z["id"]: {"isolated": False} for z in config["zones"]}, "sessions": sessions,
            "incidents": {}, "log": []}


def _on_list(config, session) -> bool:
    """A session is approved when the access list has its user for its target."""
    return any(a["user"] == session["user"] and a["target"] == session["target"]
               for a in config.get("access_list", []))


def add_session(config, state, session) -> str:
    state["sessions"][session["id"]] = {**session, "status": "active", "approved": _on_list(config, session)}
    return session["id"]


def note(state, action: str, **what) -> dict:
    """Appends to the log; the entry's `n` orders it."""
    entry = {"n": len(state["log"]) + 1, "action": action, **what}
    state["log"].append(entry)
    return entry


def session_status(state, s) -> str:
    """`revoked`, `blocked` (its target zone is isolated) or `active`."""
    if s["status"] == "revoked":
        return "revoked"
    return "blocked" if state["zones"][s["target"]]["isolated"] else "active"


def live_sessions(state, zid: str) -> list:
    """The sessions into `zid` not revoked, unknown ones and approved ones alike."""
    return [s["id"] for s in state["sessions"].values() if s["target"] == zid and s["status"] != "revoked"]


def conduit_doc(state, c) -> dict:
    cut = state["zones"][c["from"]]["isolated"] or state["zones"][c["to"]]["isolated"]
    return {**c, "status": "blocked" if cut else "open"}


def zone_doc(config, state, zid: str) -> dict:
    z = state["zones"][zid]
    return {**zone_meta(config, zid), **z, "sessions": [s["id"] for s in state["sessions"].values()
                                                         if s["target"] == zid]}


def session_doc(state, s) -> dict:
    return {**s, "status": session_status(state, s)}
