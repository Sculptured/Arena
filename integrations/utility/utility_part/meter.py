"""The site meter on the arena clock. The plant's load is the last reading the plant sent (`w`, whole watts) and
is integrated in whole watt-milliseconds, so the energy after an advance doesn't depend on how the advances were
split (docs/protocol.md, "Clock contract"). Each demand-response window is measured against its target the same
way: the time and energy inside it, and the time over the target."""
from datetime import datetime, timedelta

from arena_part import errors
from arena_part.params import config_datetime

MINUTE_MS = 60_000
WMS_PER_KWH = 3_600_000_000  # watt-milliseconds in a kilowatt-hour
DEFAULT_START = "2026-03-10T06:00:00"
DEFAULT_KW = 1500.0
DEFAULT_TARIFF = {"standard_per_kwh": 0.11, "peak_per_kwh": 0.42}


def watts(kw) -> int:
    return round(float(kw) * 1000)


def check_config(config) -> None:
    config_datetime(config, "plant_start", DEFAULT_START)
    kw = config.get("initial_kw", DEFAULT_KW)
    if not isinstance(kw, (int, float)) or isinstance(kw, bool) or kw < 0:
        raise errors.invalid(f"initial_kw must be a number of kW, 0 or more, got {kw!r}")
    tariff = config.get("tariff", DEFAULT_TARIFF)
    if not isinstance(tariff, dict) or not all(isinstance(tariff.get(k), (int, float)) for k in DEFAULT_TARIFF):
        raise errors.invalid("tariff needs standard_per_kwh and peak_per_kwh (USD per kWh)")


def new_meter(config) -> dict:
    w = watts(config.get("initial_kw", DEFAULT_KW))
    return {"w": w, "peak_w": w, "wms": {"standard": 0, "peak": 0}}


def plant_time(config, ms: int) -> str:
    start: datetime = config_datetime(config, "plant_start", DEFAULT_START)
    return (start + timedelta(milliseconds=ms)).isoformat()


def price(config, period: str) -> float:
    return float(config.get("tariff", DEFAULT_TARIFF)[f"{period}_per_kwh"])


def _inside(ev, a: int, b: int) -> bool:
    return ev["start_ms"] <= a and b <= ev["end_ms"]


def _accumulate(state, a: int, b: int) -> None:
    """The load from `a` to `b` (no window starts or ends in between)."""
    dt, w = b - a, state["meter"]["w"]
    windows = [ev for ev in state["events"].values() if _inside(ev, a, b)]
    state["meter"]["wms"]["peak" if windows else "standard"] += w * dt
    for ev in windows:
        ev["ms_in"] += dt
        ev["wms_in"] += w * dt
        ev["max_w"] = max(ev["max_w"], w)
        if w > ev["target_w"]:
            ev["ms_over"] += dt


def status_at(ev, clock_ms: int) -> str:
    if clock_ms < ev["start_ms"]:
        return "notified"
    return "active" if clock_ms < ev["end_ms"] else "ended"


def _transitions(config, state) -> list:
    """The events of windows whose status the clock just changed."""
    out = []
    for ev in state["events"].values():
        now = status_at(ev, state["clock_ms"])
        if now != ev["status"]:
            ev["status"] = now
            out.append(_window_event(config, ev))
    return out


def _window_event(config, ev) -> dict:
    target = ev["target_w"] / 1000
    if ev["status"] == "active":
        text = (f"Demand-response window {ev['id']} started: hold the site at or below {target:g} kW until "
                f"{plant_time(config, ev['end_ms'])}")
    else:
        text = f"Demand-response window {ev['id']} ended"
    return {"kind": f"demand_response_{ev['status']}", "text": text,
            "data": {"event": ev["id"], "status": ev["status"], "target_kw": target}}


def run_to(config, state, to_ms: int) -> list:
    """Integrates the meter up to `to_ms`, cut at every window boundary; gives the window events on the way.
    A clock behind the meter's changes nothing."""
    out = []
    while state["clock_ms"] < to_ms:
        t = state["clock_ms"]
        cuts = [b for ev in state["events"].values() for b in (ev["start_ms"], ev["end_ms"]) if t < b < to_ms]
        nxt = min([to_ms, *cuts])
        _accumulate(state, t, nxt)
        state["clock_ms"] = nxt
        out += _transitions(config, state)
    return out


def read(state, kw) -> None:
    """A new reading from the plant: the load from now on."""
    w = watts(kw)
    state["meter"]["w"] = w
    state["meter"]["peak_w"] = max(state["meter"]["peak_w"], w)


def kwh(wms: int) -> float:
    return round(wms / WMS_PER_KWH, 3)


def meter_doc(config, state) -> dict:
    m = state["meter"]
    period = "peak" if any(ev["status"] == "active" for ev in state["events"].values()) else "standard"
    cost = sum(m["wms"][p] / WMS_PER_KWH * price(config, p) for p in ("standard", "peak"))
    return {"meter": config.get("meter", "Site main meter"), "site": config.get("site", "Plant"),
            "at": plant_time(config, state["clock_ms"]), "kw": m["w"] / 1000, "peak_kw": m["peak_w"] / 1000,
            "kwh_this_shift": kwh(m["wms"]["standard"] + m["wms"]["peak"]),
            "energy_cost_usd": round(cost, 2), "tariff_period": period, "price_per_kwh": price(config, period)}


def event_doc(config, ev) -> dict:
    mins = ev["ms_in"] / MINUTE_MS
    avg = None if not ev["ms_in"] else round(ev["wms_in"] / ev["ms_in"] / 1000, 1)
    return {"id": ev["id"], "program": ev["program"], "target_kw": ev["target_w"] / 1000,
            "notice_at": plant_time(config, ev["notice_ms"]), "starts_at": plant_time(config, ev["start_ms"]),
            "ends_at": plant_time(config, ev["end_ms"]), "minutes": (ev["end_ms"] - ev["start_ms"]) // MINUTE_MS,
            "status": ev["status"], "price_per_kwh": price(config, "peak"),
            "acknowledged": ev["response"] is not None, "response": ev["response"],
            "minutes_in_window": round(mins, 2), "avg_kw_in_window": avg,
            "minutes_over_target": round(ev["ms_over"] / MINUTE_MS, 2),
            "max_kw_in_window": ev["max_w"] / 1000 if ev["ms_in"] else None}
