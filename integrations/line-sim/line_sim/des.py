"""A compact discrete-event simulation of serial flow lines that share one maintenance crew.

Each line is a chain of stations with finite buffers between them (blocking after service). A station
works one unit at a time; its processing time depends on its crew, the line speed and the variant,
with random variation. Machines fail on busy time (MTBF) and wait for a technician from the shared
crew to repair them (MTTR). Switching variant costs a changeover. Planned maintenance (PM) stops a
line at the start of each shift until a technician has done it; more PM means fewer failures. Units
can fail the line's test and are scrapped.

Deterministic: every random draw comes from a named stream seeded by (seed, replication, line,
station, purpose), so replication r of two configs sees the same failures, repair times and
variation wherever the configs agree (common random numbers).
"""
import heapq
import math
import random
from collections import deque

HOUR = 3600.0
EPS = 1e-9


def stream(seed, rep, *tags) -> random.Random:
    """A random stream named by its tags; str seeds hash with SHA-512, stable across processes."""
    return random.Random("/".join(map(str, (seed, rep, *tags))))


def lognormal(rng: random.Random, mean: float, cv: float) -> float:
    """A lognormal draw with this mean and coefficient of variation (`mean` itself when cv is 0)."""
    if cv <= 0:
        rng.random()  # keep the stream aligned whatever the cv
        return mean
    sigma = math.sqrt(math.log(1 + cv * cv))
    return mean * math.exp(rng.gauss(-sigma * sigma / 2, sigma))


def pm_factor(pm_hours: float) -> float:
    """How much longer machines run between failures with this much PM per week (1.0 at 4 h)."""
    return (0.5 + 1.0 * (1 - math.exp(-pm_hours / 5))) / (0.5 + 1.0 * (1 - math.exp(-4 / 5)))


class Job:
    __slots__ = ("variant", "t_in")

    def __init__(self, variant, t_in):
        self.variant = variant
        self.t_in = t_in


class Station:
    """One station's spec, its streams, its unit in work and the time it spent in each state."""

    def __init__(self, sim, line, idx, spec):
        self.sim, self.line, self.idx, self.spec = sim, line, idx, spec
        tags = (line.spec["id"], spec["id"])
        self.rng_proc = stream(sim.seed, sim.rep, *tags, "proc")
        self.rng_fail = stream(sim.seed, sim.rep, *tags, "fail")
        self.rng_repair = stream(sim.seed, sim.rep, *tags, "repair")
        self.rng_quality = stream(sim.seed, sim.rep, *tags, "quality")
        self.crew = line.crew[idx]
        self.job = None
        self.remaining = 0.0
        self.co_left = 0.0
        self.done = self.down = self.hold = False
        self.seg_start = None
        self.ver = 0
        self.last_variant = None
        self.failures = 0
        self.ttf = self._draw_ttf()
        self.times = dict.fromkeys(("working", "changeover", "starved", "blocked", "down", "pm"), 0.0)
        self.since, self.state = 0.0, "starved"

    # ---- timing ----

    def _mtbf_s(self):
        mtbf = self.spec.get("mtbf_h")
        if not mtbf:
            return math.inf
        speed = self.line.speed if self.spec.get("speed_sensitive") else 1.0
        return mtbf * HOUR * pm_factor(self.line.spec["pm_hours"]) / speed ** 2

    def _draw_ttf(self):
        return self.rng_fail.expovariate(1.0) * self._mtbf_s()

    def _proc_time(self, variant):
        base = self.spec["base_s"].get(variant, self.spec["base_s"].get("*"))
        factor = self.spec["crew_factor"][str(self.crew)]
        speed = self.line.speed if self.spec.get("speed_sensitive") else 1.0
        return lognormal(self.rng_proc, base * factor / speed, self.spec.get("cv", 0.0))

    def _changeover(self, variant):
        co = self.spec.get("changeover_min")
        if not co or self.last_variant in (None, variant):
            return 0.0
        return co[str(self.crew)] * 60.0

    def scrap_prob(self):
        q = self.spec.get("scrap")
        if not q:
            return 0.0
        over = max(0.0, self.line.spec["speed_pct"] - 100.0)
        return q["base"] + q.get("per_pct_over_100", 0.0) * over

    # ---- state ----

    def current_state(self):
        if self.hold:
            return "pm"
        if self.down:
            return "down"
        if self.job is None:
            return "starved"
        if self.done:
            return "blocked"
        return "changeover" if self.co_left > EPS else "working"

    def begin(self):
        """Before a change: count the time in the state so far and the work done in the segment."""
        t = self.sim.t
        self.times[self.state] += t - self.since
        self.since = t
        if self.seg_start is not None:
            el = t - self.seg_start
            if self.co_left > EPS:
                self.co_left = max(0.0, self.co_left - el)
            else:
                self.remaining -= el
                self.ttf -= el
            self.seg_start = None
            self.ver += 1

    def end(self):
        """After a change: the new state, and the next segment when the station can work."""
        self.state = self.current_state()
        if self.state not in ("working", "changeover"):
            return
        self.seg_start = self.sim.t
        dt = self.co_left if self.co_left > EPS else min(self.remaining, self.ttf)
        self.sim.at(self.sim.t + max(dt, 0.0), self._segment_end, self.ver)

    def accept(self, job):
        self.job = job
        self.remaining = self._proc_time(job.variant)
        self.co_left = self._changeover(job.variant)
        self.last_variant = job.variant

    def _segment_end(self, ver):
        if ver != self.ver:
            return
        self.begin()
        if self.co_left > EPS:
            self.end()
        elif self.remaining <= EPS:
            self.sim.finish(self)
        elif self.ttf <= EPS:
            self.sim.fail(self)
        else:
            self.end()


class Line:
    """A line's spec with the config applied, its stations and buffers, and what it produced."""

    def __init__(self, sim, spec):
        self.sim, self.spec = sim, spec
        self.speed = spec["speed_pct"] / 100.0
        self.crew = spec["crew"]
        self.buffers = [deque() for _ in spec["buffers"]]
        self.stations = [Station(sim, self, i, s) for i, s in enumerate(spec["stations"])]
        self.plan = self._campaigns()
        self.good = self.scrap = 0
        self.wip = 0
        self.wip_area, self.wip_since = 0.0, 0.0
        self.flow_sum, self.flow_n = 0.0, 0
        self.campaign_starts = []
        self.pm_done_s = 0.0

    def _campaigns(self):
        """The release plan: each variant's batch in turn, sized by its share of the mix."""
        variants = self.spec["variants"]
        lead = variants[0]["share"]
        sizes = [(v["id"], max(1, round(self.spec["batch_size"] * v["share"] / lead))) for v in variants]
        return {"sizes": sizes, "i": 0, "left": sizes[0][1], "first": True}

    def next_variant(self):
        p = self.plan
        if p["left"] == 0:
            p["i"] = (p["i"] + 1) % len(p["sizes"])
            p["left"] = p["sizes"][p["i"]][1]
            p["first"] = True
        if p["first"] and p["i"] == 0:
            self.campaign_starts.append(self.sim.t)
        p["first"] = False
        p["left"] -= 1
        return p["sizes"][p["i"]][0]

    def wip_change(self, d):
        t = self.sim.t
        self.wip_area += self.wip * (t - self.wip_since)
        self.wip_since = t
        self.wip += d


class Sim:
    """One replication of the plant over a warm-up and a measured week."""

    def __init__(self, plant, seed, rep):
        self.plant, self.seed, self.rep = plant, seed, rep
        self.t = 0.0
        self.heap, self.seq = [], 0
        self.techs = plant["maintenance"]["techs"]
        self.techs_busy = 0
        self.queue = deque()
        self.lines = [Line(self, spec) for spec in plant["lines"]]

    def at(self, t, fn, *args):
        heapq.heappush(self.heap, (t, self.seq, fn, args))
        self.seq += 1

    # ---- flow ----

    def pull(self, st):
        """An empty station takes its next unit: the release plan for the first, its buffer after."""
        if st.job is not None:
            return
        line = st.line
        if st.idx == 0:
            st.begin()
            st.accept(Job(line.next_variant(), self.t))
            line.wip_change(+1)
            st.end()
            return
        buf = line.buffers[st.idx - 1]
        if not buf:
            return
        st.begin()
        st.accept(buf.popleft())
        st.end()
        up = line.stations[st.idx - 1]
        if up.done:
            self.push(up)

    def finish(self, st):
        """A unit is done at `st` (its segment already counted): scrapped at a test, else passed on."""
        if st.rng_quality.random() < st.scrap_prob():
            st.line.scrap += 1
            st.line.wip_change(-1)
            self._clear(st)
            return
        st.done = True
        self.push(st)

    def push(self, st):
        """A finished unit leaves `st` if there is room downstream; `st` then takes its next one."""
        line = st.line
        last = st.idx == len(line.stations) - 1
        if not last:
            buf = line.buffers[st.idx]
            nxt_empty = not buf and line.stations[st.idx + 1].job is None
            if len(buf) >= line.spec["buffers"][st.idx] and not nxt_empty:
                st.begin()
                st.end()  # stays blocked
                return
            buf.append(st.job)
        else:
            line.good += 1
            line.wip_change(-1)
            line.flow_sum += self.t - st.job.t_in
            line.flow_n += 1
        self._clear(st)
        if not last:
            self.pull(line.stations[st.idx + 1])

    def _clear(self, st):
        st.begin()
        st.job, st.done = None, False
        st.end()
        self.pull(st)

    # ---- failures, repairs and planned maintenance ----

    def fail(self, st):
        st.down = True
        st.failures += 1
        st.ttf = st._draw_ttf()
        st.end()
        mttr = st.spec["mttr_min"] * 60.0
        self.queue.append(("repair", st, lognormal(st.rng_repair, mttr, 0.6)))
        self.dispatch()

    def start_pm(self, line, hours):
        for st in line.stations:
            st.begin()
            st.hold = True
            st.end()
        self.queue.appendleft(("pm", line, hours * HOUR))  # planned work goes first
        self.dispatch()

    def dispatch(self):
        while self.queue and self.techs_busy < self.techs:
            kind, who, duration = self.queue.popleft()
            self.techs_busy += 1
            self.at(self.t + duration, self._work_done, kind, who, duration)

    def _work_done(self, kind, who, duration):
        self.techs_busy -= 1
        stations = [who] if kind == "repair" else who.stations
        if kind == "pm":
            who.pm_done_s += duration
        for st in stations:
            st.begin()
            if kind == "repair":
                st.down = False
            else:
                st.hold = False
            st.end()
        self.dispatch()

    # ---- the run ----

    def _reset_stats(self):
        """At the end of the warm-up: what the week measures starts from zero."""
        for line in self.lines:
            line.wip_change(0)
            line.good = line.scrap = 0
            line.wip_area, line.flow_sum, line.flow_n = 0.0, 0.0, 0
            line.campaign_starts = []
            for st in line.stations:
                st.begin()
                st.end()
                st.times = dict.fromkeys(st.times, 0.0)
                st.failures = 0

    def run(self):
        week = self.plant["week"]
        warmup, shift = week["warmup_hours"] * HOUR, week["shift_hours"] * HOUR
        horizon = warmup + week["shifts"] * shift
        self.at(warmup, self._reset_stats)
        for k in range(week["shifts"]):
            for line in self.lines:
                pm = line.spec["pm_hours"] / week["shifts"]
                if pm > 0:
                    self.at(warmup + k * shift + EPS, self.start_pm, line, pm)
        for line in self.lines:
            self.pull(line.stations[0])
        while self.heap and self.heap[0][0] <= horizon:
            self.t, _, fn, args = heapq.heappop(self.heap)
            fn(*args)
        self.t = horizon
        for line in self.lines:
            line.wip_change(0)
            for st in line.stations:
                st.begin()
        return horizon - warmup


def replicate(plant: dict, seed: int, rep: int) -> dict:
    """One replication's raw results: per line, units made and scrapped, WIP, flow and states."""
    sim = Sim(plant, seed, rep)
    measured = sim.run()
    return {"measured_s": measured, "lines": [_line_result(line, measured) for line in sim.lines]}


def _line_result(line, measured):
    starts = line.campaign_starts
    cycle = (starts[-1] - starts[0]) / (len(starts) - 1) if len(starts) > 1 else None
    return {
        "id": line.spec["id"], "good": line.good, "scrap": line.scrap,
        "wip_mean": line.wip_area / measured,
        "flow_mean_s": line.flow_sum / line.flow_n if line.flow_n else None,
        "campaign_cycle_s": cycle, "pm_s": line.pm_done_s,
        "stations": [{"id": st.spec["id"], "failures": st.failures,
                      "share": {k: v / measured for k, v in st.times.items()}} for st in line.stations],
    }
