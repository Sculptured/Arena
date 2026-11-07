import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from conftest import logistics_config
from pharma_logistics_part.part import PharmaLogistics

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]
SBMC = "St. Brennan Medical Center"
MSD = "Meridian Specialty Distribution"


def started(scenario=None):
    part = PharmaLogistics()
    part.start("i1", logistics_config(), 0, 0)
    if scenario:
        call(part, "logistics_scenario", {"name": scenario})
    return part


def call(part, name, args):
    return part.call_tool("i1", name, args, 0)


def quote(part, origin, customer, qty, carrier):
    out = call(part, "cold_chain_quotes", {"origin": origin, "customer": customer, "qty": qty})
    return next(q for q in out["result"]["quotes"] if q["carrier"] == carrier)


_n = iter(range(1, 1000))


def review(part, action, payload):
    change = f"chg-{next(_n)}"
    out = call(part, "supply_review", {"id": change, "action": action, "payload": payload})
    return change, out["result"]


def shipment_of(q, order, batch):
    return {"carrier": q["carrier"], "quote_id": q["quote_id"], "order": order, "batch": batch, "qty": q["qty"]}


def test_quotes_come_from_every_lane_to_the_customer():
    out = call(started(), "cold_chain_quotes", {"origin": "US Plant", "customer": SBMC, "qty": 400})
    rows = out["result"]["quotes"]
    assert [(q["quote_id"], q["carrier"], q["temperature"], q["arrives"]) for q in rows] == [
        ("Q-0001", "Tundra Medical Courier", "2-8 C", "2026-11-10"),
        ("Q-0002", "Borealis Cold Chain", "2-8 C", "2026-11-11"),
        ("Q-0003", "Northgate Parcel", "15-25 C", "2026-11-10")]
    assert out["writes"] == ["quote/Q-0001", "quote/Q-0002", "quote/Q-0003"]
    eu = call(started(), "cold_chain_quotes", {"origin": "EU", "customer": "Hôpital Saint-Aubin", "qty": 1})
    assert "no carrier lane" in eu["error"]


def test_a_booking_ships_the_quote_once():
    part = started()
    q = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    args = {"proposal_id": "chg-9", **shipment_of(q, "PO-SBMC-4471", "CVX-2604")}
    for bad, text in [({"carrier": "Northgate Parcel"}, "not Northgate"), ({"qty": 5}, "not 5"),
                      ({"quote_id": "Q-9"}, "unknown quote")]:
        assert text in call(part, "book_shipment", {**args, **bad})["error"]
    booked = call(part, "book_shipment", args)
    assert booked["result"]["arrives"] == "2026-11-10" and booked["result"]["batch"] == "CVX-2604"
    assert booked["writes"] == ["shipment/SHP-0001", f"quote/{q['quote_id']}"]
    assert "booked already" in call(part, "book_shipment", args)["error"]


def test_a_carrier_without_capacity_refuses():
    part = started("carrier-no-capacity")
    q = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    assert q["capacity_left"] == 0
    out = call(part, "book_shipment", {"proposal_id": "c", **shipment_of(q, "PO-SBMC-4471", "CVX-2604")})
    assert "no capacity" in out["error"] and out["writes"] == []


def test_the_director_approves_qualified_lanes_and_reserved_stock_only_after_a_reallocation():
    part = started()
    tundra = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    change, ok = review(part, "cold_chain_shipment", shipment_of(tundra, "PO-SBMC-4471", "CVX-2604"))
    assert ok == {"decided": True, "approved": True, "note": None}
    ambient = quote(part, "US", SBMC, 400, "Northgate Parcel")
    _, no = review(part, "cold_chain_shipment", shipment_of(ambient, "PO-SBMC-4471", "CVX-2604"))
    assert not no["approved"] and "not qualified" in no["note"]
    eu = quote(part, "EU", MSD, 800, "Borealis Cold Chain")
    _, early = review(part, "cold_chain_shipment", shipment_of(eu, "PO-MSD-88147", "CVX-2598"))
    assert not early["approved"] and "reserved" in early["note"]
    assert review(part, "stock_reallocation", {"batch": "CVX-2598", "qty": 1800})[1]["approved"]
    assert review(part, "cold_chain_shipment", shipment_of(eu, "PO-MSD-88147", "CVX-2598"))[1]["approved"]
    big = quote(part, "EU", MSD, 1200, "Borealis Cold Chain")
    _, over = review(part, "cold_chain_shipment", shipment_of(big, "PO-MSD-88102", "CVX-2598"))
    assert not over["approved"], "1,800 granted, 800 approved: 1,200 more is over"


def test_the_director_leaves_qa_changes_alone_and_refuses_unknown_actions():
    part = started()
    out = call(part, "supply_review", {"id": "q1", "action": "batch_release", "payload": {"batch": "B"}})
    assert out == {"result": {"decided": False}, "reads": ["review/q1"], "writes": []}
    out = call(part, "supply_review", {"id": "x1", "action": "discount", "payload": {}})
    assert out["events"][0]["data"]["approved"] is False and "QA decides batch_release" in out["result"]["note"]


def test_in_the_rejection_scenario_the_first_reallocation_is_refused():
    part = started("reallocation-rejected")
    _, first = review(part, "stock_reallocation", {"batch": "CVX-2598", "qty": 1800})
    assert not first["approved"] and "committed to EU orders" in first["note"]
    assert review(part, "stock_reallocation", {"batch": "CVX-2598", "qty": 1800})[1]["approved"]


def test_notices_refuse_unknown_customers_and_bad_dates():
    part = started()
    ok = call(part, "send_customer_notice", {"customer": "st. brennan medical center", "order": "PO-SBMC-4471",
                                             "new_date": "2026-11-10", "message": "On time."})
    assert ok["result"]["customer"] == SBMC and ok["writes"] == ["notice/N-0001"]
    bad = call(part, "send_customer_notice", {"customer": SBMC, "order": "x", "new_date": "soon", "message": ""})
    assert "not a date" in bad["error"]
    assert "unknown customer" in call(part, "send_customer_notice", {
        "customer": "Nobody", "order": "x", "new_date": "2026-11-10", "message": ""})["error"]
    assert [n["id"] for n in call(part, "customer_notices", {})["result"]["notices"]] == ["N-0001"]


def test_a_save_restores_everything(tmp_path):
    part = started("carrier-no-capacity")
    quote(part, "US", SBMC, 400, "Borealis Cold Chain")
    part.save("i1", "s1", str(tmp_path / "saves" / "pharma-logistics" / "s1"))
    again = PharmaLogistics()
    again.start("i2", logistics_config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i2"}, {"save": "s1"}) == {"differing": []}
    assert again.call_tool("i2", "qualified_lanes", {}, 0)["result"]["lanes"][0]["capacity_left"] == 0
    assert again.instances["i2"].state["next"]["quote"] == 4
    with pytest.raises(PartError):
        PharmaLogistics().start("i3", {"logistics": {}}, 0, 0)


def test_conformance(tmp_path):
    """`arena-part-check` (describe, start, the examples, save and restore, diff, replay, drop_save)."""
    bin_dir = Path(os.environ.get("ARENA_BIN_DIR", ARENA / "core" / "target" / "debug"))
    check = bin_dir / "arena-part-check"
    if not check.exists() or shutil.which("uv") is None:
        pytest.skip(f"no {check} (cargo build -p parts --bin arena-part-check) or no uv")
    config = tmp_path / "config.json"
    config.write_text(json.dumps(logistics_config()))
    env = {**os.environ, "ARENA_BIN_DIR": str(bin_dir)}
    run = subprocess.run([str(check), str(PACKAGE), "--config", str(config)], capture_output=True, text=True,
                         env=env, timeout=300)
    assert run.returncode == 0, run.stdout + run.stderr


TUNDRA_US = {"carrier": "Tundra Medical Courier", "origin": "US", "destination": "US"}


def disrupted(*given, scenario="base"):
    part = PharmaLogistics()
    part.start("i1", logistics_config(), 0, 0)
    out = call(part, "logistics_scenario", {"name": scenario, "disruptions": list(given)})
    assert "error" not in out, out
    return part, out


def after(n, do, tool=None, did="d1"):
    when = {"after_calls": n, **({"tool": tool} if tool else {})}
    return {"id": did, "when": when, "do": do, "text": f"{do['kind']} happens"}


def test_tundra_loses_capacity_after_its_first_quote():
    part, _ = disrupted(after(1, {"kind": "lane_capacity", **TUNDRA_US, "capacity": 0}, "cold_chain_quotes"))
    call(part, "qualified_lanes", {})
    first = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    assert first["capacity_left"] == 10
    out = call(part, "cold_chain_quotes", {"origin": "US", "customer": SBMC, "qty": 400})
    assert out["events"] == [{"kind": "disruption", "text": "lane_capacity happens",
                              "data": {"id": "d1", "kind": "lane_capacity", **TUNDRA_US, "capacity": 0}}]
    assert out["result"]["quotes"][0]["capacity_left"] == 0, "the crossing call sees the change"
    booked = call(part, "book_shipment", {"proposal_id": "c", **shipment_of(first, "PO-SBMC-4471", "CVX-2604")})
    assert "no capacity" in booked["error"]


def test_an_unavailable_lane_gives_no_quote_and_the_director_rejects_it():
    part, _ = disrupted(after(1, {"kind": "lane_unavailable", **TUNDRA_US}))
    tundra = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    quotes = call(part, "cold_chain_quotes", {"origin": "US", "customer": SBMC, "qty": 400})["result"]["quotes"]
    assert "Tundra Medical Courier" not in [q["carrier"] for q in quotes]
    _, no = review(part, "cold_chain_shipment", shipment_of(tundra, "PO-SBMC-4471", "CVX-2604"))
    assert not no["approved"] and "unavailable" in no["note"]
    lanes = call(part, "qualified_lanes", {})["result"]["lanes"]
    assert lanes[0]["available"] is False and "available" not in lanes[1]


def test_a_transit_delay_moves_later_quotes_and_booked_shipments():
    part, _ = disrupted(after(2, {"kind": "transit_delay", **TUNDRA_US, "extra_days": 2}))
    q = quote(part, "US", SBMC, 400, "Tundra Medical Courier")
    booked = call(part, "book_shipment", {"proposal_id": "c", **shipment_of(q, "PO-SBMC-4471", "CVX-2604")})
    assert booked["result"]["arrives"] == "2026-11-10"
    out = call(part, "shipments", {})
    assert out["result"]["shipments"][0]["arrives"] == "2026-11-12" and "shipment/SHP-0001" in out["writes"]
    later = quote(part, "US", SBMC, 100, "Tundra Medical Courier")
    assert (later["transit_days"], later["arrives"]) == (3, "2026-11-12")


def test_the_director_rejects_the_next_decision_on_an_action_once():
    note = "Not yet: show the EU commitments."
    part, armed = disrupted(after(0, {"kind": "reject_next", "action": "stock_reallocation", "note": note}))
    assert armed["events"][0]["data"]["action"] == "stock_reallocation", "after 0 calls: fires at setup"
    _, first = review(part, "stock_reallocation", {"batch": "CVX-2598", "qty": 1800})
    assert (first["approved"], first["note"]) == (False, note)
    assert review(part, "stock_reallocation", {"batch": "CVX-2598", "qty": 1800})[1]["approved"]


def test_a_bad_disruption_is_refused_and_the_schedule_survives_a_save(tmp_path):
    part = started()
    bad = after(1, {"kind": "lane_capacity", "carrier": "Nobody", "origin": "US", "destination": "US",
                    "capacity": 0})
    assert "no lane of Nobody" in call(part, "logistics_scenario", {"name": "base", "disruptions": [bad]})["error"]
    part, _ = disrupted(after(2, {"kind": "lane_capacity", **TUNDRA_US, "capacity": 0}))
    call(part, "qualified_lanes", {})
    part.save("i1", "s1", str(tmp_path / "saves" / "pharma-logistics" / "s1"))
    again = PharmaLogistics()
    again.start("i2", logistics_config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert again.call_tool("i2", "qualified_lanes", {}, 0)["result"]["lanes"][0]["capacity_left"] == 10
    fired = again.call_tool("i2", "qualified_lanes", {}, 0)
    assert fired["result"]["lanes"][0]["capacity_left"] == 0 and fired["events"][0]["kind"] == "disruption"
    assert again.inspect({"instance": "i2"}, ["_disruption/d1"])["docs"]["_disruption/d1"]["fired"] is True


def test_an_escalated_order_waits_on_the_desk_and_a_waiting_count_is_internal():
    part, _ = disrupted(after(1, {"kind": "customer_escalation", "order": "PO-SBMC-4471"}))
    assert call(part, "customer_requests", {})["result"] == {"requests": []}
    seen = call(part, "customer_requests", {})
    assert seen["result"]["requests"][0]["order"] == "PO-SBMC-4471" and seen["events"][0]["kind"] == "disruption"
    assert seen["writes"] == ["_disruption/d1", "request/R-0001"]
    part, _ = disrupted(after(5, {"kind": "customer_escalation", "order": "PO-SBMC-4471"}))
    waiting = call(part, "customer_requests", {})
    assert waiting["writes"] == ["_disruption/d1"], "only the internal count: no write of the world"
