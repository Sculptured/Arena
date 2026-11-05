"""What the showcase scene shows moving, computed from the plant's readings, plain Python: blanks and formed parts on
the belts, the press's visible shake, the forklift's round. Nothing here feeds back into the plant: Factory and
PlantIO never see these values, so the scenario's timing and the scoring are the same with or without them.

A press cycle is `cycle`: the crank's turns, unwrapped, with 0 at top dead centre and 0.5 at bottom dead centre."""
import math
from dataclasses import dataclass, field

# Phases of a press cycle (top dead centre = 0) at which the line's handling acts. The die is open from about 0.79
# to 0.21; the loader enters last and leaves first.
PHASE_LOADER_GRIP = 0.38
PHASE_LOADER_RELEASE = 0.0
PHASE_STAMP = 0.5
PHASE_UNLOADER_GRIP = 0.85
PHASE_UNLOADER_RELEASE = 0.12

BELT_SPEED = 0.38  # m/s
BLANK_GAP = 1.05  # m, centre to centre when blanks stand queued
PART_GAP = 0.9
SHAKE_FROM_MM_S = 2.3  # no visible shake below ISO 10816-3 zone B
SHAKE_FULL_MM_S = 11.2  # the trip level
SHAKE_MAX_M = 0.03  # visible amplitude at the trip: the real one is a fraction of a millimetre
WARM_S = 6.0  # the drives spin up from rest; their readings mean nothing before this


@dataclass
class Belt:
    """Parts on a belt, from `start` to `stop` (positions along it). They run at BELT_SPEED until they queue."""
    start: float
    stop: float
    gap: float
    parts: list = field(default_factory=list)  # distances along the belt, the first is the furthest

    def advance(self, dt: float) -> None:
        limit = self.stop
        for i, x in enumerate(self.parts):
            self.parts[i] = min(limit, x + BELT_SPEED * dt)
            limit = self.parts[i] - self.gap

    def has_room(self) -> bool:
        return not self.parts or self.parts[-1] - self.start >= self.gap

    def at_stop(self) -> bool:
        return bool(self.parts) and self.parts[0] >= self.stop - 1e-6


@dataclass
class LineFlow:
    """The parts of one press line as its press cycles: the washer feeds blanks onto the infeed belt on a timer; the
    loader takes the first to the die; the die forms it at bottom dead centre; the unloader puts it on the outfeed
    belt, which ends in the stillage. When the press stops, the infeed queues up until it is full, the outfeed runs
    empty and the stillage stops growing."""
    feed_period: float  # seconds between blanks at the press's nominal speed
    infeed: Belt
    outfeed: Belt
    loader_has: bool = True
    die: str | None = "blank"  # None | "blank" | "formed"
    unloader_has: bool = False
    stillage: int = 0
    strokes: int = 0
    cycle: float | None = None
    next_feed: float = 0.0
    now: float = 0.0

    def update(self, cycle: float, dt: float) -> None:
        """Moves on by `dt` seconds with the crank now at `cycle`."""
        self.now += dt
        if self.cycle is not None and cycle > self.cycle:
            self._cross(self.cycle, cycle)
        self.cycle = cycle
        if self.outfeed.at_stop():
            self.outfeed.parts.pop(0)
            self.stillage += 1
        self.infeed.advance(dt)
        self.outfeed.advance(dt)
        if self.now >= self.next_feed and self.infeed.has_room():
            self.infeed.parts.append(self.infeed.start)
            self.next_feed = self.now + self.feed_period

    def _cross(self, before: float, after: float) -> None:
        for at, act in sorted((turn + phase, act) for turn in range(int(before) - 1, int(after) + 2)
                              for phase, act in _ACTIONS):
            if before < at <= after:
                act(self)

    def warm(self, cycles: int) -> None:
        """A run-in before the first frame, so that the line is already in full production when the video starts."""
        dt = self.feed_period / 60
        cycle = self.cycle if self.cycle is not None else 0.5
        self.cycle = cycle
        for _ in range(int(cycles * 60)):
            cycle += 1 / 60
            self.update(cycle, dt)
        self.cycle = None  # the first real reading starts from wherever the crank really is

    def _grip_blank(self) -> None:
        if not self.loader_has and self.infeed.at_stop():
            self.infeed.parts.pop(0)
            self.loader_has = True

    def _release_blank(self) -> None:
        if self.loader_has and self.die is None:
            self.loader_has, self.die = False, "blank"

    def _stamp(self) -> None:
        if self.die == "blank":
            self.die = "formed"
            self.strokes += 1

    def _grip_formed(self) -> None:
        if self.die == "formed" and not self.unloader_has:
            self.die, self.unloader_has = None, True

    def _release_formed(self) -> None:
        if self.unloader_has and self.outfeed.has_room():
            self.unloader_has = False
            self.outfeed.parts.append(self.outfeed.start)


_ACTIONS = ((PHASE_LOADER_GRIP, LineFlow._grip_blank), (PHASE_LOADER_RELEASE, LineFlow._release_blank),
            (PHASE_STAMP, LineFlow._stamp), (PHASE_UNLOADER_GRIP, LineFlow._grip_formed),
            (PHASE_UNLOADER_RELEASE, LineFlow._release_formed))


def crank_cycle(shaft_angle: float, crank_ratio: float) -> float:
    """The press cycle from the shaft's unwrapped angle: the factory's crank is shaft / ratio + pi, 0 at the top."""
    return shaft_angle / crank_ratio / (2 * math.pi) + 0.5


def shake_amplitude(vibration_mm_s: float, now: float) -> float:
    """How far the press visibly shakes, in metres, for its measured vibration: nothing in zone A, the full amount at
    the trip level. It exaggerates the real motion on purpose, and only ever follows the reading."""
    if now < WARM_S:
        return 0.0
    f = (vibration_mm_s - SHAKE_FROM_MM_S) / (SHAKE_FULL_MM_S - SHAKE_FROM_MM_S)
    return SHAKE_MAX_M * min(1.0, max(0.0, f)) ** 1.3


def shake_offset(amplitude: float, now: float) -> tuple:
    """The press's displacement at time `now`: incommensurate tones, so it never reads as a loop."""
    return (amplitude * (0.6 * math.sin(83.0 * now) + 0.4 * math.sin(127.0 * now + 1.0)),
            amplitude * (0.7 * math.sin(97.0 * now + 2.0) + 0.3 * math.sin(151.0 * now)),
            0.5 * amplitude * math.sin(71.0 * now + 0.5))


@dataclass(frozen=True)
class Leg:
    """One leg of a kinematic round: from `a` to `b` over `seconds`, with the forks at the end heights. A point may
    carry a third value, a yaw in degrees: a leg turns from a's yaw to b's (a turn on the spot when a and b coincide);
    without one, it faces along the leg."""
    a: tuple
    b: tuple
    seconds: float
    fork0: float = 0.1
    fork1: float = 0.1
    carrying: bool = False
    reverse: bool = False  # drives backwards: faces b -> a


class Round:
    """A vehicle's looped route: pose(now) -> x, y, yaw in degrees, fork height and whether a load is on the forks."""

    def __init__(self, legs):
        self.legs = list(legs)
        self.total = sum(leg.seconds for leg in self.legs)

    def pose(self, now: float) -> tuple:
        t = now % self.total
        for leg in self.legs:
            if t < leg.seconds:
                return _on_leg(leg, t / leg.seconds)
            t -= leg.seconds
        return _on_leg(self.legs[-1], 1.0)


def _on_leg(leg: Leg, f: float) -> tuple:
    s = f * f * (3 - 2 * f)
    dx, dy = leg.b[0] - leg.a[0], leg.b[1] - leg.a[1]
    x, y = leg.a[0] + dx * s, leg.a[1] + dy * s
    heading = math.degrees(math.atan2(dy, dx)) + (180.0 if leg.reverse else 0.0)
    yaw0 = leg.a[2] if len(leg.a) > 2 else heading
    yaw1 = leg.b[2] if len(leg.b) > 2 else heading
    yaw = yaw0 + ((yaw1 - yaw0 + 180.0) % 360.0 - 180.0) * s  # turns the short way
    return x, y, yaw, leg.fork0 + (leg.fork1 - leg.fork0) * s, leg.carrying
