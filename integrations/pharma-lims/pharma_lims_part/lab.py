"""The lab's records as the config gives them, seen through the instance's scenario: environmental-monitoring
samples, each batch's release tests, and the documents. Everything here only reads."""
from datetime import date

from arena_part import errors
from arena_part.params import Refusal

BASE = "base"


def check_config(config) -> None:
    """The config needs `lims` (samples, tests, test names, documents, scenarios), `lots` and `today`."""
    lims = config.get("lims")
    if not isinstance(lims, dict) or not isinstance(config.get("lots"), list):
        raise errors.invalid("config needs lims and lots (the benchmark's data)")
    for key in ("em_samples", "tests", "test_names", "documents"):
        if key not in lims:
            raise errors.invalid(f"config.lims has no {key}")
    try:
        date.fromisoformat(config.get("today") or "")
    except ValueError:
        raise errors.invalid("config.today must be a date (YYYY-MM-DD)") from None


def scenarios(config) -> list:
    return [BASE, *sorted(config["lims"].get("scenarios", {}))]


def overlay(config, state) -> dict:
    """What the instance's scenario changes (empty for the base)."""
    return config["lims"].get("scenarios", {}).get(state["scenario"], {})


def sample_status(s) -> str:
    if s["cfu"] > s["action_limit"]:
        return "action limit exceeded"
    alert = s.get("alert_limit")
    if alert is not None and s["cfu"] > alert:
        return "alert limit exceeded"
    return "within limits"


def samples(config, state) -> list:
    comments = overlay(config, state).get("sample_comments", {})
    out = []
    for s in config["lims"]["em_samples"]:
        doc = {**s, "status": sample_status(s)}
        if s["id"] in comments:
            doc["comment"] = comments[s["id"]]
        out.append(doc)
    return out


def excursions(config, state, batch) -> list:
    """The ids of the batch's samples above their action limit."""
    return [s["id"] for s in samples(config, state) if s["batch"] == batch and s["status"] == "action limit exceeded"]


def lot(config, batch) -> dict:
    found = next((l for l in config["lots"] if l["batch"] == batch), None)
    if found is None:
        raise Refusal(f"no batch {batch} in the LIMS")
    return found


def batch_results(config, state, batch) -> dict:
    """The batch's release tests (one status and date for all, as the data gives them) and its excursions."""
    the_lot = lot(config, batch)
    tests = {**config["lims"]["tests"], **overlay(config, state).get("tests", {})}
    status, when = tests.get(batch, ["pending", None])
    note = overlay(config, state).get("pending_note", {}).get(batch)
    rows = [{"test": name, "status": status, "date": when} for name in config["lims"]["test_names"]]
    if note:
        rows[0] = {**rows[0], "note": note}
    out = {"batch": batch, "manufactured": the_lot["manufactured"], "expiry": the_lot["expiry"],
           "tests": rows, "complete": status != "pending", "all_passed": status == "pass",
           "em_excursions": excursions(config, state, batch)}
    failed = state.get("failed", {}).get(batch)
    return _failed(config, out, failed) if failed else out


def _failed(config, results, test) -> dict:
    """`results` once a disruption failed `test`: that result is out of specification."""
    for i, row in enumerate(results["tests"]):
        if row["test"] == test:
            results["tests"][i] = {"test": test, "status": "fail", "date": config["today"],
                                   "note": "Retest out of specification"}
    return {**results, "complete": True, "all_passed": False, "failed": [test]}


def documents(config, state) -> list:
    return [*config["lims"]["documents"], *overlay(config, state).get("documents", [])]


def document(config, state, doc_id) -> dict:
    found = next((d for d in documents(config, state) if d["id"] == doc_id), None)
    if found is None:
        raise Refusal(f"no document {doc_id}")
    return found
