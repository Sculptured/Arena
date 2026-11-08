"""One instance's plant: the factory's Factory on the arena clock, stepped at a fixed physics rate."""
from datetime import datetime, timedelta, timezone

from arena_part import errors

from . import recording, views
from .equipment import Equipment
from .log import log


class ArenaClock:
    """What the names `time` and `datetime` are inside the factory module (the factory itself is never edited):
    both read the arena clock of the plant being driven, so the plant clock Factory.reset takes, the promised
    dates and each event's `wall` are the same in every run. One per process; a Plant points it at itself
    before calling into its Factory."""

    fromisoformat = staticmethod(datetime.fromisoformat)

    def __init__(self):
        self.plant = None

    def now(self) -> datetime:
        return self.plant.plant_now()

    def time(self) -> float:
        return self.now().replace(tzinfo=timezone.utc).timestamp()

    monotonic = time


CLOCK = ArenaClock()


def physics_hz(cfg, mods) -> int:
    """The configured rate, else the factory's own press_model.PHYSICS_HZ."""
    return cfg.physics_hz or mods.press_model.PHYSICS_HZ


class Plant:
    def __init__(self, mods, backend, cfg, clock_ms: int, video_path=None):
        """A Factory on `backend` with erp and triggers None, its threads never started: the part's thread calls
        it directly. With `video_path`, the backend renders the plant there as it steps."""
        self.backend, self.cfg = backend, cfg
        self.hz = physics_hz(cfg, mods)
        self.steps = clock_ms * self.hz // 1000  # physics steps done, counted from arena time 0
        self.last_seq = 0  # the last factory event `advance` returned
        mods.factory.time = mods.factory.datetime = CLOCK
        data = mods.press_model.load_data(cfg.data_path)
        CLOCK.plant = self
        self.io = backend.make(cfg, data, self.hz)
        self.factory = mods.factory.Factory(self.io, data, log=log)
        self.equipment = Equipment(cfg.cells, cfg.power, self.hz)
        self.seed = None
        _check_metered(self.factory, cfg.power)
        self.video = None if video_path is None else backend.video(cfg, self.io, self.hz, video_path)

    def plant_now(self) -> datetime:
        return self.cfg.plant_start + timedelta(seconds=self.steps / self.hz)

    def act(self, fn):
        """fn(factory) under the factory's lock, on this plant's clock."""
        CLOCK.plant = self
        with self.factory.lock:
            return fn(self.factory)

    def begin(self, seed: int, scenario: bool) -> None:
        """The arena seed seeds the factory's randomness (the floor noise); start_scenario resets with it too."""
        self.seed = seed
        self.act(lambda f: f.start_scenario(seed) if scenario else f.reset(seed))

    def set_scenario(self, name: str) -> dict:
        """`press-wear`: PR-A1's wear starts now (as `scenario: true` does at the start); `calm`: the plant is reset
        with the run's seed, so no press wears."""
        if name not in SCENARIOS:
            raise ValueError(f"scenario is one of {', '.join(SCENARIOS)}")
        self.act(lambda f: f.start_scenario(self.seed) if name == "press-wear" else f.reset(self.seed))
        return {"scenario": name, "plant": self.factory.scenario}

    def _physics_step(self, dt: float) -> None:
        """After each backend step. Factory.physics_step takes the factory's lock itself, as in run.py."""
        self.steps += 1
        self.factory.physics_step(dt)
        with self.factory.lock:
            self.equipment.step(self.steps, self.factory)

    def step_to(self, clock_ms: int) -> int:
        """Physics steps of 1/hz s up to `clock_ms` (none when it is behind), then the factory's queued work.
        Events stay queued for `advance`. Returns the number of steps."""
        target = clock_ms * self.hz // 1000
        n = max(0, target - self.steps)
        CLOCK.plant = self
        if n:
            self._run_to(target)
        self.factory.drain()
        return n

    def _run_to(self, target: int) -> None:
        """Physics steps up to step `target`. With a recording, a frame at each frame boundary on the way. Steps
        that rendering itself makes still reach Factory.physics_step and count, so the clock stays right."""
        if self.video is None:
            self._run(target - self.steps)
            return
        every = self.video.every
        while self.steps < target:
            self._run(min(target, recording.next_frame(self.steps, every)) - self.steps)
            if self.steps % every == 0:
                self.video.frame(self)

    def _run(self, n: int) -> None:
        """`n` backend steps; each must reach Factory.physics_step, or the plant would silently stand still."""
        before = self.steps
        self.backend.run(self.io, n, 1.0 / self.hz, self._physics_step)
        if self.steps < before + n:
            raise errors.internal(f"{n} physics steps ran but only {self.steps - before} reached the plant: the "
                                  "physics callback is not running")

    def stop(self) -> None:
        if self.video is not None:
            self.video.close()
        self.backend.stop(self.io)

    def new_events(self) -> list:
        """Factory events since the last call, as arena events."""
        events = [e for e in self.factory.events if e["seq"] > self.last_seq]
        if events:
            self.last_seq = events[-1]["seq"]
        return [views.arena_event(e) for e in events]

    def advance(self, clock_ms: int) -> dict:
        n = self.step_to(clock_ms)
        events = self.new_events()
        return {"changed": bool(n or events), "events": events}

    def state(self) -> dict:
        """The factory's state, with the cells and the load the arena adds."""
        return self.act(lambda f: {**f.state(), **self.equipment.state(f)})


SCENARIOS = ("press-wear", "calm")


def _check_metered(factory, power) -> None:
    unknown = sorted(set((power or {}).get("lines", {})) - set(factory.lines_meta))
    if unknown:
        raise errors.invalid(f"power names lines the plant doesn't have: {', '.join(unknown)}")
