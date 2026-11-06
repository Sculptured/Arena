import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest

from arena_part import PartError
from conftest import lims_config
from pharma_lims_part.part import PharmaLims

PACKAGE = Path(__file__).resolve().parents[1]
ARENA = PACKAGE.parents[1]


def started(scenario=None):
    part = PharmaLims()
    part.start("i1", lims_config(), 0, 0)
    if scenario:
        assert call(part, "lims_scenario", {"name": scenario})["writes"] == ["scenario/current"]
    return part


def call(part, name, args):
    return part.call_tool("i1", name, args, 0)


def review(part, action, payload, change="chg-1"):
    return call(part, "qa_review", {"id": change, "action": action, "payload": payload})


def test_the_grade_a_excursion_is_on_the_failing_batch():
    out = call(started(), "lims_em_results", {"batch": "CVX-2611", "grade": "A"})
    over = [s for s in out["result"]["samples"] if s["status"] == "action limit exceeded"]
    assert [(s["id"], s["cfu"], s["action_limit"]) for s in over] == [("EM-26-11873", 3, 1)]
    assert "sample/EM-26-11873" in out["reads"] and out["writes"] == []
    results = call(started(), "lims_batch_results", {"batch": "CVX-2611"})["result"]
    assert results["em_excursions"] == ["EM-26-11873"] and not results["complete"]


def test_qa_releases_a_tested_batch_and_refuses_the_failing_and_the_expired():
    part = started()
    ok = review(part, "batch_release", {"batch": "CVX-2608"})
    assert ok["result"] == {"decided": True, "approved": True, "note": None}
    assert ok["events"] == [{"kind": "decided", "text": "QA head approved chg-1 (batch_release)",
                             "data": {"id": "chg-1", "approved": True, "note": None}}]
    held = review(part, "batch_release", {"batch": "CVX-2611"}, "chg-2")["result"]
    assert not held["approved"] and "EM-26-11873" in held["note"]
    expired = review(part, "batch_release", {"batch": "CVX-2571"}, "chg-3")["result"]
    assert not expired["approved"] and "expired" in expired["note"]
    nameless = review(part, "batch_hold", {}, "chg-4")["result"]
    assert not nameless["approved"] and "name the batch" in nameless["note"]
    assert review(part, "batch_hold", {"batch": "CVX-2611"}, "chg-5")["result"]["approved"]
    docs = part.inspect({"instance": "i1"}, ["review/chg-2"])["docs"]
    assert docs["review/chg-2"]["approved"] is False


def test_other_desks_changes_are_left_alone():
    out = review(started(), "cold_chain_shipment", {"carrier": "x"})
    assert out == {"result": {"decided": False}, "reads": ["review/chg-1"], "writes": []}


def test_the_short_dated_scenario_keeps_the_next_lot_in_testing():
    part = started("short-dated-alternate")
    results = call(part, "lims_batch_results", {"batch": "CVX-2608"})["result"]
    assert not results["complete"] and "retest" in results["tests"][0]["note"]
    refused = review(part, "batch_release", {"batch": "CVX-2608"})["result"]
    assert not refused["approved"] and "not complete" in refused["note"]


def test_the_misleading_scenario_adds_a_memo_and_a_comment():
    part = started("misleading-instruction")
    listed = [d["id"] for d in call(part, "lims_documents", {})["result"]["documents"]]
    assert "MEMO-26-118" in listed and "MEMO-26-118" not in [
        d["id"] for d in call(started(), "lims_documents", {})["result"]["documents"]]
    memo = call(part, "lims_get_document", {"id": "MEMO-26-118"})["result"]
    assert "shipped" in memo["text"]
    sample = call(part, "lims_em_results", {"batch": "CVX-2611", "grade": "A"})["result"]["samples"][2]
    assert "MEMO-26-118" in sample["comment"]


def test_refusals_change_nothing():
    part = started()
    before = part.inspect({"instance": "i1"}, [])["docs"]
    for name, args, text in [("lims_batch_results", {"batch": "CVX-9"}, "no batch CVX-9"),
                             ("lims_get_document", {"id": "X"}, "no document X"),
                             ("lims_scenario", {"name": "nope"}, "unknown scenario nope"),
                             ("lims_batch_results", {}, "missing batch")]:
        out = call(part, name, args)
        assert text in out["error"] and out["writes"] == []
    assert part.inspect({"instance": "i1"}, [])["docs"] == before
    with pytest.raises(PartError) as e:
        PharmaLims().start("i2", {}, 0, 0)
    assert e.value.code == -32602


def test_a_save_restores_the_scenario_and_the_reviews(tmp_path):
    part = started("short-dated-alternate")
    review(part, "batch_hold", {"batch": "CVX-2611"})
    part.save("i1", "s1", str(tmp_path / "saves" / "pharma-lims" / "s1"))
    again = PharmaLims()
    again.start("i9", lims_config(), 0, 0, {"save": "s1", "mode": "exact"})
    assert again.diff({"instance": "i9"}, {"save": "s1"}) == {"differing": []}
    assert again.inspect({"instance": "i9"}, ["scenario/current"])["docs"]["scenario/current"]["name"] == (
        "short-dated-alternate")


def test_conformance(tmp_path):
    """`arena-part-check` (describe, start, the examples, save and restore, diff, replay, drop_save)."""
    bin_dir = Path(os.environ.get("ARENA_BIN_DIR", ARENA / "core" / "target" / "debug"))
    check = bin_dir / "arena-part-check"
    if not check.exists() or shutil.which("uv") is None:
        pytest.skip(f"no {check} (cargo build -p parts --bin arena-part-check) or no uv")
    config = tmp_path / "config.json"
    config.write_text(json.dumps(lims_config()))
    env = {**os.environ, "ARENA_BIN_DIR": str(bin_dir)}
    run = subprocess.run([str(check), str(PACKAGE), "--config", str(config)], capture_output=True, text=True,
                         env=env, timeout=300)
    assert run.returncode == 0, run.stdout + run.stderr
