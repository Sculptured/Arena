"""The carriers' network as the config gives it, seen through the instance's scenario: lanes, their
qualification for 2-8 C, capacity left this week, customers and dates. Everything here only reads."""
from datetime import date, timedelta

from arena_part import errors
from arena_part.params import Refusal

BASE = "base"


def check_config(config) -> None:
    """The config needs `logistics` (origins, lanes, scenarios), `customers` and `today`."""
    net = config.get("logistics")
    if not isinstance(net, dict) or not isinstance(config.get("customers"), list):
        raise errors.invalid("config needs logistics and customers (the benchmark's data)")
    for key in ("origins", "lanes"):
        if key not in net:
            raise errors.invalid(f"config.logistics has no {key}")
    try:
        date.fromisoformat(config.get("today") or "")
    except ValueError:
        raise errors.invalid("config.today must be a date (YYYY-MM-DD)") from None


def scenarios(config) -> list:
    return [BASE, *sorted(config["logistics"].get("scenarios", {}))]


def overlay(config, state) -> dict:
    return config["logistics"].get("scenarios", {}).get(state["scenario"], {})


def origin_key(config, named: str) -> str:
    """`US` for "US", "us plant" or the origin's full name."""
    for key, name in config["logistics"]["origins"].items():
        if named.strip().casefold() in (key.casefold(), name.casefold()) or (
                named.strip() and name.casefold().startswith(named.strip().casefold())):
            return key
    known = "; ".join(f"{k} ({v})" for k, v in config["logistics"]["origins"].items())
    raise Refusal(f"unknown origin {named!r} (origins: {known})")


def customer(config, name: str) -> dict:
    found = next((c for c in config["customers"] if c["name"].casefold() == name.strip().casefold()), None)
    if found is None:
        raise Refusal(f"unknown customer {name!r}")
    return found


def lanes(config, origin: str, region: str) -> list:
    return [l for l in config["logistics"]["lanes"] if l["origin"] == origin and l["destination"] == region]


def lane_of(config, carrier: str, origin: str):
    return next((l for l in config["logistics"]["lanes"] if l["carrier"] == carrier and l["origin"] == origin), None)


def capacity_left(config, state, lane) -> int:
    """The lane's shipments left this week: its capacity (or the scenario's) less what is booked on it."""
    capacity = overlay(config, state).get("capacity", {}).get(lane["carrier"], lane["capacity"])
    booked = [s for s in state["shipments"].values()
              if s["carrier"] == lane["carrier"] and s["origin"] == lane["origin"]]
    return max(capacity - len(booked), 0)


def days_after(config, days: int) -> str:
    return (date.fromisoformat(config["today"]) + timedelta(days=days)).isoformat()


def iso_date(text: str) -> str:
    try:
        return date.fromisoformat(text).isoformat()
    except (TypeError, ValueError):
        raise Refusal(f"{text!r} is not a date (YYYY-MM-DD)") from None
