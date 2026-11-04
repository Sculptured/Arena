import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from ridgeline_hub_part.part import RidgelineHub

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]
HARTWELL = "Hartwell Industrial Supply"
ALL_KEYS = []  # inspect with no keys returns every key


def started(clock_ms=0, instance="i1", part=None, **config):
    part = part or RidgelineHub()
    part.start(instance, config, 1, clock_ms)
    return part


def call(part, name, args, clock_ms, instance="i1"):
    return part.call_tool(instance, name, args, clock_ms)


def docs(part, instance="i1"):
    return part.inspect({"instance": instance}, ALL_KEYS)["docs"]


def quoted(events, supplier=HARTWELL):
    return [e for e in events if f"{supplier} quoted" in e["text"]]


def test_quotes_arrive_on_arena_clock():
    part = started()
    rfq = call(part, "request_quotes", {"part": "SP-SPN-800", "qty": 1}, 0)
    assert rfq["result"]["rfq"] == "RFQ-001"
    assert rfq["writes"] == ["rfq/RFQ-001"]
    early = call(part, "get_quotes", {"rfq": "RFQ-001"}, 17_000)
    assert early["result"]["quotes"] == []
    assert early["reads"] == ["rfq/RFQ-001"]
    assert quoted(part.advance("i1", 17_000)["events"]) == []
    late = call(part, "get_quotes", {"rfq": "RFQ-001", "wait_s": 90}, 18_000)
    assert [q["supplier"] for q in late["result"]["quotes"]] == [HARTWELL]
    events = part.advance("i1", 18_000)["events"]
    assert len(quoted(events)) == 1
    assert quoted(events)[0]["kind"] == "market"
    assert part.advance("i1", 60_000)["events"] and not quoted(part.advance("i1", 90_000)["events"])


def test_booking_gives_a_shipment_booked_event():
    part = started()
    freight = call(part, "freight_quotes", {"origin": "Dock", "weight_kg": 10}, 0)["result"]["request"]
    book = {"proposal_id": "P", "quote_id": f"{freight}-1", "carrier": "Lakeshore Freight", "amount": 420}
    call(part, "book_freight", book, 22_000)
    events = part.advance("i1", 22_500)["events"]
    booked = [e for e in events if e["kind"] == "shipment_booked"]
    assert len(booked) == 1
    data = booked[0]["data"]
    assert (data["part"], data["origin"], data["carrier"], data["transit_hours"], data["ref"]) == (
        "SP-SPN-800", "Dock", "Lakeshore Freight", 72, f"{freight}-1")


def test_freight_booking_records_shipment():
    part = started()
    call(part, "freight_quotes", {"origin": "Columbus OH", "weight_kg": 50}, 0)
    some = call(part, "get_freight_quotes", {"request": "FRQ-001"}, 10_000)
    assert len(some["result"]["quotes"]) == 1
    full = call(part, "get_freight_quotes", {"request": "FRQ-001"}, 22_000)
    assert len(full["result"]["quotes"]) == 3
    book = {"proposal_id": "P1", "quote_id": "FRQ-001-1", "carrier": "Lakeshore Freight", "amount": 420}
    first = call(part, "book_freight", book, 22_000)
    ship = first["result"]["shipment"]
    assert ship["id"] == "SHP-001"
    assert ship["arrives"] == "2026-03-13T06:00:22"
    assert first["writes"] == ["freight/FRQ-001", "shipment/SHP-001"]
    again = call(part, "book_freight", book, 23_000)
    assert again["result"]["already_booked"] is True
    assert "shipment/SHP-001" not in again["writes"]
    d = docs(part)
    assert d["shipment/SHP-001"] == ship
    assert d["freight/FRQ-001"]["proposal_id"] == "P1"


def test_refusals_are_tool_errors_that_change_nothing():
    part = started()
    call(part, "freight_quotes", {"origin": "Columbus OH", "weight_kg": 50}, 0)
    before = docs(part)
    good = {"proposal_id": "p", "quote_id": "FRQ-001-1", "carrier": "Lakeshore Freight", "amount": 420}
    cases = [
        ("get_quotes", {"rfq": "RFQ-9"}, "unknown rfq RFQ-9"),
        ("answer_claim", {"claim": "GM-CLM-1", "answers": []}, "GM-CLM-1"),
        ("award_order", {"rfq": "RFQ-9", "supplier": HARTWELL, "po": "PO-1"}, "RFQ-9"),
        ("book_freight", {**good, "quote_id": "FRQ-009-1"}, "FRQ-009"),
        ("book_freight", {**good, "quote_id": "FRQ-001-0"}, "FRQ-001-0"),
        ("book_freight", {**good, "carrier": "Buckeye Expedite"}, "carrier"),
        ("book_freight", {**good, "amount": 421}, "amount"),
        ("request_quotes", {"qty": 1}, "missing part"),
        ("request_quotes", {"part": "x", "qty": "many"}, "qty"),
    ]
    for name, args, text in cases:
        out = call(part, name, args, 0)
        assert text in out["error"] and out["reads"] == [] and out["writes"] == [] and "result" not in out
    assert docs(part) == before
    ok = call(part, "book_freight", {**good, "carrier": " lakeshore freight ", "amount": 420.001}, 0)
    assert ok["result"]["booked"] is True


def test_unknown_tool_is_invalid():
    with pytest.raises(PartError) as e:
        call(started(), "no_such_tool", {}, 0)
    assert e.value.code == -32602


def test_award_with_unknown_supplier_changes_nothing():
    part = started()
    call(part, "request_quotes", {"part": "SP-SPN-800"}, 0)
    out = call(part, "award_order", {"rfq": "RFQ-001", "supplier": "Nobody Ltd", "po": "PO-1"}, 0)
    assert "Nobody Ltd" in out["error"]
    assert docs(part)["rfq/RFQ-001"]["awarded"] is None
    ok = call(part, "award_order", {"rfq": "RFQ-001", "supplier": HARTWELL, "po": "PO-1"}, 0)
    assert ok["result"]["confirmed"] is True and ok["writes"] == ["rfq/RFQ-001"]


def first_half(part, instance="i1"):
    call(part, "request_quotes", {"part": "SP-SPN-800", "qty": 2}, 0, instance)
    call(part, "freight_quotes", {"origin": "Columbus OH", "weight_kg": 50}, 5_000, instance)
    call(part, "notify_customer", {"proposal_id": "P2", "customer": "Northfield Motors", "order": "SO-1",
                                   "new_date": "2026-03-20", "message": "late"}, 12_000, instance)
    call(part, "submit_claim", {"policy": "POL-1", "machine": "PR-A1", "amount": 9000,
                                "downtime_hours": 30, "evidence": {"log": "x"}}, 20_000, instance)
    call(part, "get_quotes", {"rfq": "RFQ-001"}, 30_000, instance)  # steps to 30 s, events still queued


def second_half(part, instance="i1"):
    out = []
    book = {"proposal_id": "P3", "quote_id": "FRQ-001-2", "carrier": "Interstate Hotshot", "amount": 780}
    out.append(call(part, "book_freight", book, 40_000, instance))
    out.append(call(part, "answer_claim", {"claim": "GM-CLM-4471", "answers": ["a", "b"]}, 45_000, instance))
    out.append(call(part, "award_order", {"rfq": "RFQ-001", "supplier": HARTWELL, "po": "PO-7"}, 60_000, instance))
    out.append(call(part, "customer_replies", {}, 80_000, instance))
    return out, part.advance(instance, 90_000)["events"]


def save_dir(saves_root, save):
    return str(saves_root / "ridgeline-hub" / save)


def restored(saves_root, save="s1"):
    part = RidgelineHub()
    part.start("b", {}, 1, 30_000, {"save": save, "mode": "exact"})
    return part


def test_save_restore_equal_future(saves_root):
    a = started()
    first_half(a)
    saved = a.save("i1", "s1", save_dir(saves_root, "s1"))
    assert saved["exact"] is True and saved["bytes"] > 0
    a_calls, a_events = second_half(a)
    b = restored(saves_root)
    assert docs(b, "b") == a.inspect({"save": "s1"}, [])["docs"] != docs(a)
    b_calls, b_events = second_half(b, "b")
    assert a_events and b_events == a_events
    assert b_calls == a_calls
    assert docs(b, "b") == docs(a)
    assert a.diff({"instance": "i1"}, {"instance": "i1"}) == {"differing": []}
    assert a.diff({"save": "s1"}, {"instance": "i1"})["differing"]


def test_replay_equals_save(saves_root):
    a = started()
    first_half(a)
    a.save("i1", "s1", save_dir(saves_root, "s1"))
    b = restored(saves_root)
    b_calls, b_events = second_half(b, "b")
    replay = started(instance="r")
    first_half(replay, "r")
    r_calls, r_events = second_half(replay, "r")
    assert (r_calls, r_events) == (b_calls, b_events)
    assert docs(replay, "r") == docs(b, "b")


def test_announced_tuples_roundtrip(saves_root):
    a = started()
    call(a, "request_quotes", {"part": "SP-SPN-800"}, 0)
    call(a, "freight_quotes", {"origin": "Columbus OH", "weight_kg": 5}, 0)
    a.advance("i1", 20_000)
    market = a.instances["i1"].market
    assert (("RFQ-001", HARTWELL) in market.announced) and ("FRQ-001-1" in market.announced)
    a.save("i1", "s1", save_dir(saves_root, "s1"))
    b = restored(saves_root)
    announced = b.instances["b"].market.announced
    assert announced == market.announced
    assert any(isinstance(x, tuple) for x in announced) and any(isinstance(x, str) for x in announced)
    b.advance("b", 20_000)
    assert quoted(b.advance("b", 60_000)["events"]) == []  # not announced a second time


def test_call_at_earlier_clock_changes_nothing():
    part = started()
    call(part, "request_quotes", {"part": "SP-SPN-800"}, 0)
    call(part, "get_quotes", {"rfq": "RFQ-001"}, 20_000)
    before = docs(part)
    out = call(part, "get_quotes", {"rfq": "RFQ-001"}, 5_000)
    assert [q["supplier"] for q in out["result"]["quotes"]] == [HARTWELL]  # the market stays at 20 s
    queued = part.instances["i1"].factory.events[:]
    assert part.advance("i1", 5_000)["events"] == queued
    assert docs(part) == before and part.instances["i1"].elapsed_ms() == 20_000


def test_start_from_without_save_is_invalid():
    with pytest.raises(PartError) as e:
        RidgelineHub().start("b", {}, 1, 0, {"mode": "exact"})
    assert e.value.code == -32602


def hub_copy(tmp_path, change=None):
    """A temp copy of the factory hub and its data, optionally with `change` appended to market.py."""
    from ridgeline_hub_part.config import DEFAULT_HUB
    root = tmp_path / "factory"
    shutil.copytree(DEFAULT_HUB, root / "hub", ignore=shutil.ignore_patterns("__pycache__", ".venv", "tests"))
    (root / "data").mkdir()
    shutil.copy(DEFAULT_HUB.parent / "data" / "ridgeline.json", root / "data" / "ridgeline.json")
    if change:
        with open(root / "hub" / "ridgeline_hub" / "market.py", "a") as f:
            f.write(change)
    return root / "hub"


def test_fingerprint_hashes_market_and_data():
    fp = RidgelineHub().describe()["fingerprint"]
    assert set(fp["sha256"]) == {"market.py", "ridgeline.json"}
    assert all(len(h) == 64 for h in fp["sha256"].values())
    assert fp["adapter"] == 1 and fp["tick_ms"] == 500


def test_changed_market_changes_fingerprint_and_is_refused(tmp_path):
    from ridgeline_hub_part.config import parse_config, source_hashes
    same_copy = hub_copy(tmp_path / "a")
    started(hub_path=str(same_copy))  # identical content is accepted
    changed = hub_copy(tmp_path / "b", "\n# changed\n")
    hashes = source_hashes(parse_config({"hub_path": str(changed)}))
    assert hashes["market.py"] != RidgelineHub().describe()["fingerprint"]["sha256"]["market.py"]
    with pytest.raises(PartError) as e:
        started(hub_path=str(changed))
    assert e.value.code == -32003 and "market.py" in e.value.message and str(changed) in e.value.message


def test_refused_start_imports_nothing(tmp_path, monkeypatch):
    import sys
    for name in [m for m in sys.modules if m.split(".")[0] == "ridgeline_hub"]:
        monkeypatch.delitem(sys.modules, name)
    monkeypatch.setattr(sys, "path", list(sys.path))
    changed = hub_copy(tmp_path, "\n# changed\n")
    with pytest.raises(PartError) as e:
        started(hub_path=str(changed))
    assert e.value.code == -32003
    assert "ridgeline_hub.market" not in sys.modules and str(changed) not in sys.path


def test_restore_from_missing_save_is_unknown():
    with pytest.raises(PartError) as e:
        RidgelineHub().start("b", {}, 1, 0, {"save": "nope", "mode": "exact"})
    assert e.value.code == -32002


def test_missing_hub_path_is_clear(tmp_path):
    gone = tmp_path / "no-such-hub"
    with pytest.raises(PartError) as e:
        started(hub_path=str(gone))
    assert e.value.code == -32602
    assert str(gone) in e.value.message
    with pytest.raises(PartError, match="plant_start"):
        started(plant_start="tomorrow-ish")


def test_gc_drops_saves_not_kept(saves_root):
    a = started()
    for s in ("base_1", "tmp_1"):
        a.save("i1", s, save_dir(saves_root, s))
    out = a.gc(["base_*"])
    assert out["dropped"] == 1 and out["bytes"] > 0
    assert (saves_root / "ridgeline-hub" / "base_1").is_dir()
    assert not (saves_root / "ridgeline-hub" / "tmp_1").exists()
    assert a.usage() == {"bytes": 0}


def test_describe_declares_clock_restore_and_seed():
    d = RidgelineHub().describe()
    assert d["uses_clock"] is True and d["restore"] == {"exact": "save", "approximate": False}
    assert "seed" in str(d["fingerprint"])
    names = {t["name"] for t in d["tools"]}
    assert names == {"request_quotes", "get_quotes", "award_order", "freight_quotes", "get_freight_quotes",
                     "book_freight", "submit_claim", "answer_claim", "notify_customer", "customer_replies"}


def checker():
    for profile in ("debug", "release"):
        p = ARENA / "core" / "target" / profile / "arena-part-check"
        if p.is_file():
            return p
    return None


def test_conformance(tmp_path):
    binary = checker()
    if binary is None or shutil.which("uv") is None:
        pytest.skip("arena-part-check is not built (cargo build -p parts --bin arena-part-check) or uv missing")
    saves = tmp_path / "check-saves"
    saves.mkdir()
    env = {**os.environ, "ARENA_SAVES": str(saves)}
    r = subprocess.run([str(binary), str(PACKAGE)], capture_output=True, text=True, env=env, timeout=300)
    assert r.returncode == 0, r.stdout + r.stderr
