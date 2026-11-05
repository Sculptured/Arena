"""The showcase's camera plan, plain Python: establishing shots while nothing is wrong, and cuts on the plant's events
during a failure. `Director.update` takes the factory's new events and arena time, and returns the shot to show and
how far through it the camera is, so a recording of the same run always cuts at the same frames."""
from dataclasses import dataclass


@dataclass(frozen=True)
class Shot:
    eye: tuple
    target: tuple
    focal: float
    eye_end: tuple | None = None  # the eye drifts here over the shot
    follow: str | None = None  # looks at this moving thing instead of `target`
    shake: float = 0.0  # how much of the press's visible shake reaches the camera


ESTABLISHING = (("wide", 8.0), ("robot_b", 7.0), ("belt_c", 7.0), ("forklift", 6.0), ("press_a_far", 7.0))
# What cuts in on the failing press, as (shot, seconds).
FIRST_WARNING = (("press_a", 8.0), ("hmi_a", 4.0))
SECOND_WARNING = (("press_a_low", 7.0), ("hmi_a", 4.0), ("press_a", 8.0))
TRIP = (("press_a", 5.0), ("beacon_a", 5.0), ("hmi_a", 4.0), ("infeed_a", 9.0), ("outfeed_a", 7.0), ("wide_a", 8.0))
AFTERMATH = (("wide_a", 8.0), ("infeed_a", 6.0), ("beacon_a", 4.0), ("outfeed_a", 5.0), ("wide", 7.0))
CUTS = {"technician": (("service_a", 8.0),), "repair": (("press_a", 8.0),), "lockout": (("service_a", 6.0),),
        "delivery": (("dock", 10.0),)}


def _cue(event: dict):
    """The shots an event calls for, or None. Only PR-A1 fails in this scenario; other machines' news stays off."""
    kind, text = event.get("kind"), event.get("text", "")
    if kind == "alarm" and "PR-A1" in text:
        return TRIP
    if kind == "warning" and "PR-A1" in text:
        return SECOND_WARNING if "zone D" in text else FIRST_WARNING
    if kind == "shipment" and (event.get("data") or {}).get("status") == "at-dock":
        return CUTS["delivery"]
    return CUTS.get(kind) if "PR-A1" in text or kind == "delivery" else None


class Director:
    """`auto` follows the plan; any other mode is a shot's name and stays on it."""

    def __init__(self, shots: dict, mode: str = "auto"):
        self.shots, self.mode = shots, mode
        self.queue = []
        self.current, self.started, self.ends = mode if mode != "auto" else "wide", 0.0, 0.0
        self.idle, self.idle_i = ESTABLISHING, 0
        self.cuts = []  # (time, shot), for the run's report

    def _start(self, name: str, seconds: float, now: float) -> None:
        self.current, self.started, self.ends = name, now, now + seconds
        self.cuts.append((round(now, 2), name))

    def _next(self, now: float) -> None:
        if self.queue:
            self._start(*self.queue.pop(0), now)
            return
        self._start(*self.idle[self.idle_i % len(self.idle)], now)
        self.idle_i += 1

    def update(self, events, now: float) -> tuple:
        """Returns (shot, fraction through it, 0..1)."""
        if self.mode != "auto":
            return self.shots[self.mode], 0.0
        for event in events:
            cue = _cue(event)
            if cue is None:
                continue
            if cue is TRIP:
                self.idle, self.idle_i = AFTERMATH, 0
            self.queue = list(cue)
            self.ends = now  # cut now
        if now >= self.ends:
            self._next(now)
        span = max(self.ends - self.started, 1e-6)
        return self.shots[self.current], min(1.0, (now - self.started) / span)


def eye_at(shot: Shot, fraction: float) -> tuple:
    if shot.eye_end is None:
        return shot.eye
    return tuple(a + (b - a) * fraction for a, b in zip(shot.eye, shot.eye_end))
