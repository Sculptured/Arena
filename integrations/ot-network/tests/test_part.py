import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from conftest import config, front
from ot_network_part.part import OtNetwork

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]


def rogue(after=0, tool=None):
    when = {"after_calls": after, **({"tool": tool} if tool else {})}
    return {"id": "rogue", "when": when, "do": {"kind": "remote_session", **front("remote_session")},
            "text": "An unknown remote session reaches CELL-R1"}


def started(*given):
    part = OtNetwork()
    part.start("i1", config(), 0, 0)
    out = call(part, "ot_scenario", {"name": "base", "disruptions": list(given)})
    assert "error" not in out, out
    return part, out


def call(part, name, args):
    return part.call_tool("i1", name, args, 0)


def doc(part, key):
    return part.inspect({"instance": "i1"}, [key])["docs"][key]


def test_the_network_shows_its_zones_conduits_and_the_known_session():
    part, _ = started()
    out = call(part, "ot_network_status", {})["result"]
    assert [z["id"] for z in out["zones"]] == ["ENTERPRISE", "DMZ", "SUPERVISORY", "PRESS-LINES", "CELL-R1"]
    assert all(c["status"] == "open" for c in out["conduits"])
    s = call(part, "remote_sessions", {})["result"]["sessions"]
    assert [(x["id"], x["approved"], x["status"]) for x in s] == [("RS-1180", True, "active")]
    assert call(part, "ot_playbook", {})["result"]["id"] == "OT-IR-01"


def test_the_unknown_session_appears_after_its_count():
    part, out = started(rogue(after=1, tool="remote_sessions"))
    assert "events" not in out
    assert len(call(part, "remote_sessions", {})["result"]["sessions"]) == 1
    second = call(part, "remote_sessions", {})
    assert second["events"][0]["kind"] == "disruption"
    s = second["result"]["sessions"][1]
    assert (s["id"], s["approved"], s["status"], s["target"]) == ("RS-7731", False, "active", "CELL-R1")


def test_the_playbook_order_is_recorded_isolate_then_revoke_then_restore():
    part, _ = started(rogue())
    assert "only a cell zone" in call(part, "isolate_zone", {"zone": "PRESS-LINES", "reason": "x"})["error"]
    iso = call(part, "isolate_zone", {"zone": "CELL-R1", "reason": "unknown session RS-7731"})
    assert iso["result"]["blocked_sessions"] == ["RS-7731"]
    assert set(iso["writes"]) == {"zone/CELL-R1", "session/RS-7731"}
    assert doc(part, "session/RS-7731")["status"] == "blocked"
    assert doc(part, "conduit/C-4")["status"] == "blocked"
    assert "isolated already" in call(part, "isolate_zone", {"zone": "CELL-R1", "reason": "x"})["error"]
    rev = call(part, "revoke_session", {"session": "RS-7731", "reason": "not on the access list"})
    assert rev["result"]["revoked_while_isolated"] is True and rev["result"]["status"] == "revoked"
    call(part, "report_security_incident", {"summary": "Unknown session", "session": "RS-7731", "zone": "CELL-R1"})
    back = call(part, "restore_zone", {"zone": "CELL-R1", "proposal_id": "chg-1"})
    assert back["result"]["unknown_sessions_live"] == []
    z = doc(part, "zone/CELL-R1")
    assert (z["isolated"], z["isolated_n"], z["restored_n"]) == (False, 1, 4)
    assert "restored_with_live_session" not in z
    assert [e["action"] for e in doc(part, "log/actions")["entries"]] == [
        "isolate_zone", "revoke_session", "report_security_incident", "restore_zone"]


def test_restoring_while_the_session_is_live_is_recorded():
    part, _ = started(rogue())
    call(part, "isolate_zone", {"zone": "CELL-R1", "reason": "x"})
    call(part, "restore_zone", {"zone": "CELL-R1", "proposal_id": "chg-1"})
    assert doc(part, "zone/CELL-R1")["restored_with_live_session"] == ["RS-7731"]
    assert doc(part, "session/RS-7731")["status"] == "active"
    assert "not isolated" in call(part, "restore_zone", {"zone": "CELL-R1", "proposal_id": "c"})["error"]


def test_revoking_without_isolating_is_not_contained():
    part, _ = started(rogue())
    rev = call(part, "revoke_session", {"session": "RS-7731", "reason": "x"})["result"]
    assert rev["revoked_while_isolated"] is False and "contained_n" not in rev
    assert "revoked already" in call(part, "revoke_session", {"session": "RS-7731", "reason": "x"})["error"]
    assert "unknown session" in call(part, "revoke_session", {"session": "RS-9", "reason": "x"})["error"]


def test_a_save_restores_everything(tmp_path):
    part, _ = started(rogue(after=1))
    call(part, "ot_network_status", {})
    call(part, "isolate_zone", {"zone": "CELL-R1", "reason": "x"})
    part.save("i1", "s1", str(tmp_path / "saves" / "ot-network" / "s1"))
    again = OtNetwork()
    again.start("i2", config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    assert again.inspect({"instance": "i2"}, ["session/RS-7731"])["docs"]["session/RS-7731"]["status"] == "blocked"
    with pytest.raises(PartError):
        OtNetwork().start("i3", {"zones": []}, 0, 0)
    bad = rogue()
    bad["do"]["target"] = "NOWHERE"
    assert "no zone" in call(part, "ot_scenario", {"name": "base", "disruptions": [bad]})["error"]


def test_conformance(tmp_path):
    """`arena-part-check` (describe, start, the examples, save and restore, diff, drop_save)."""
    bin_dir = Path(os.environ.get("ARENA_BIN_DIR", ARENA / "core" / "target" / "debug"))
    check = bin_dir / "arena-part-check"
    if not check.exists() or shutil.which("uv") is None:
        pytest.skip(f"no {check} (cargo build -p parts --bin arena-part-check) or no uv")
    path = tmp_path / "config.json"
    path.write_text(json.dumps(config()))
    env = {**os.environ, "ARENA_BIN_DIR": str(bin_dir)}
    run = subprocess.run([str(check), str(PACKAGE), "--config", str(path)], capture_output=True, text=True,
                         env=env, timeout=300)
    assert run.returncode == 0, run.stdout + run.stderr
