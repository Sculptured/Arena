"""A world as JSON: the save file, restoring from it, and the keyed documents inspect and diff show."""
import json
from pathlib import Path

from arena_part import errors

from . import inbound
from .world import World

FILE = "state.json"
MARKET_FIELDS = ("rfqs", "freight", "claims", "notices", "approved")
# inspect/diff key prefix -> where the documents are in a snapshot
KEY_KINDS = {"rfq": "rfqs", "freight": "freight", "claim": "claims", "notice": "notices",
             "shipment": "shipments"}


def _announced(items) -> list:
    """Tuples become lists in JSON; restore turns them back. Sorted so equal states write equal files."""
    return sorted((list(i) if isinstance(i, tuple) else i for i in items), key=json.dumps)


def snapshot(world: World) -> dict:
    """The whole state of a world as JSON-safe values (a copy, not a view)."""
    market = world.market
    state = {f: getattr(market, f) for f in MARKET_FIELDS}
    state.update(announced=_announced(market.announced), shipments=world.factory.shipments,
                 events=world.factory.events, clock_ms=world.elapsed_ms(), inbound=world.inbound)
    return json.loads(json.dumps(state))


def restore(world: World, state: dict) -> None:
    market = world.market
    for f in MARKET_FIELDS:
        setattr(market, f, state[f])
    market.announced = {tuple(i) if isinstance(i, list) else i for i in state["announced"]}
    world.factory.shipments = state["shipments"]
    world.factory.events = state["events"]
    world.stepper.done_ms = state["clock_ms"]
    world.inbound = state["inbound"]


def docs_of(state: dict) -> dict:
    """Every inspectable key of a snapshot, `<kind>/<id>`, with its document."""
    out = {f"{kind}/{id_}": doc for kind, field in KEY_KINDS.items() for id_, doc in state[field].items()}
    return {**out, **inbound.docs(state["inbound"], state["clock_ms"])}


def write(dir, state: dict) -> int:
    body = json.dumps(state, separators=(",", ":")).encode()
    Path(dir).mkdir(parents=True, exist_ok=True)
    (Path(dir) / FILE).write_bytes(body)
    return len(body)


def read(save_dir) -> dict:
    try:
        return json.loads((Path(save_dir) / FILE).read_bytes())
    except OSError:
        raise errors.unknown(f"unknown save {Path(save_dir).name}") from None
