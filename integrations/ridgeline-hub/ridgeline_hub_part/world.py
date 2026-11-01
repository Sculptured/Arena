"""One instance's world: the hub's Market on the arena clock, with an adapter for its `factory`."""
from datetime import datetime, timedelta

from arena_part import FixedStep

from .config import Config, load_market

TICK_MS = 500  # the hub ticks the market every 0.5 s


class ArenaFactory:
    """What the Market expects of the hub's factory: an event queue, a plant clock and shipments.
    Shipments are recorded here only; they are not forwarded to Isaac Sim (no cross-part handoff in v1)."""

    def __init__(self, plant_start: datetime, elapsed_ms):
        self.plant_start = plant_start
        self.elapsed_ms = elapsed_ms
        self.events: list[dict] = []
        self.shipments: dict[str, dict] = {}

    def emit(self, kind, source, text, data):
        self.events.append({"kind": kind, "text": text, "data": {**data, "source": source}})

    def clock(self) -> datetime:
        return self.plant_start + timedelta(milliseconds=self.elapsed_ms())

    def add_shipment(self, part, origin, carrier, transit_hours, ref) -> dict:
        arrives = (self.clock() + timedelta(hours=transit_hours)).replace(microsecond=0).isoformat()
        sid = f"SHP-{len(self.shipments) + 1:03d}"
        shipment = {"id": sid, "part": part, "origin": origin, "carrier": carrier,
                    "transit_hours": transit_hours, "ref": ref, "arrives": arrives}
        self.shipments[sid] = shipment
        return shipment

    def drain(self) -> list[dict]:
        events, self.events = self.events, []
        return events


def arena_market_class(base):
    """`base` (the hub's Market) with `now()` reading arena seconds instead of the wall clock."""

    class ArenaMarket(base):
        def __init__(self, factory, suppliers, elapsed_ms):
            super().__init__(factory, suppliers)
            self.elapsed_ms = elapsed_ms

        def now(self) -> float:
            return self.elapsed_ms() / 1000

    return ArenaMarket


class World:
    def __init__(self, cfg: Config, clock_ms: int = 0):
        module, suppliers = load_market(cfg)
        self.module = module
        self.stepper = FixedStep(TICK_MS)
        self.stepper.done_ms = clock_ms - clock_ms % TICK_MS
        self.factory = ArenaFactory(cfg.plant_start, self.elapsed_ms)
        self.market = arena_market_class(module.Market)(self.factory, suppliers, self.elapsed_ms)

    def elapsed_ms(self) -> int:
        return self.stepper.done_ms

    def step_to(self, clock_ms: int) -> None:
        """Ticks the market once per whole step up to `clock_ms`; events stay queued."""
        self.stepper.advance_to(clock_ms, lambda _step: self.market.tick())

    def advance(self, clock_ms: int) -> list[dict]:
        self.step_to(clock_ms)
        return self.factory.drain()
