"""Plant 2's assembly floor as data and a projection: three lines, a robot cell, a motor stock, a power cap
and overtime nights. Time is frozen at the planning point (Monday 06:00); the plant's MES projects what the
current setpoints produce over the horizon, in 15-minute steps, deterministically."""
import math

STEP_MIN = 15
DAYS = ["Mon", "Tue", "Wed", "Thu"]
DAY_START_H, DAY_END_H = 6, 22  # two shifts a day; a night runs only on overtime
MODES = ("AUTO", "MANUAL", "MANUAL_UNTIL_R1")


class PlanError(ValueError):
    """A setpoint the MES can't take (an unknown order, a product the line can't make)."""


def minutes(label: str) -> int:
    """'Tue 06:00' -> minutes after Mon 06:00."""
    try:
        day, hm = label.split()
        h, m = (int(x) for x in hm.split(":"))
        return DAYS.index(day) * 1440 + (h - DAY_START_H) * 60 + m
    except ValueError:
        raise PlanError(f"a time must look like 'Tue 06:00', got {label!r}") from None


def label(t: int) -> str:
    d, rem = divmod(t + DAY_START_H * 60, 1440)
    return f"{DAYS[d]} {rem // 60:02d}:{rem % 60:02d}"


def new_state(config) -> dict:
    lines = {ln["id"]: {"queue": list(ln["queue"]), "mode": ln["mode"], "start_at": "Mon 06:00",
                        "product": ln.get("product")} for ln in config["lines"]}
    return {"scenario": "base", "orders": [dict(o) for o in config["orders"]], "lines": lines,
            "overtime": {n: False for n in config["overtime"]["nights"]},
            "cell": {"fault": None, "loto": False, "repair_requested": False},
            "motors_on_hand": config["motors_on_hand"], "denials": [], "log": [], "events": []}


def order(state, oid):
    for o in state["orders"]:
        if o["id"] == oid:
            return o
    raise PlanError(f"no order {oid!r} (orders: {', '.join(o['id'] for o in state['orders'])})")


def parse_entry(state, entry: str):
    """'SO-2001' or 'SO-2001:60' -> (order, qty or None)."""
    oid, _, qty = str(entry).partition(":")
    o = order(state, oid.strip())
    if qty and (not qty.strip().isdigit() or int(qty) <= 0):
        raise PlanError(f"a queue entry is 'SO-2001' or 'SO-2001:60', got {entry!r}")
    return o, int(qty) if qty else None


def check_line(config, state, lid) -> None:
    meta = line_meta(config, lid)
    for e in state["lines"][lid]["queue"]:
        o, _ = parse_entry(state, e)
        if o["product"] not in meta["makes"]:
            raise PlanError(f"{meta['name']} can't make {o['product']} ({o['id']})")
    if state["lines"][lid]["mode"] not in MODES:
        raise PlanError(f"mode must be one of {', '.join(MODES)}")
    minutes(state["lines"][lid]["start_at"])


def line_meta(config, lid):
    for ln in config["lines"]:
        if ln["id"] == lid:
            return ln
    raise PlanError(f"no line {lid!r}")


def in_shift(config, state, t) -> bool:
    if t >= config["horizon_h"] * 60:
        return False
    day, rem = divmod(t, 1440)
    if rem < (DAY_END_H - DAY_START_H) * 60:
        return True
    night = DAYS[day].lower()
    return state["overtime"].get(night, False)


def cell_ready_at(config, state):
    c = state["cell"]
    if not c["fault"]:
        return 0
    return config["cell"]["repair_h"] * 60 if c["repair_requested"] else None


class _Line:
    def __init__(self, config, state, meta):
        self.meta, self.sp = meta, state["lines"][meta["id"]]
        self.entries = [[o, q] for o, q in (parse_entry(state, e) for e in self.sp["queue"])]
        self.product = self.sp.get("product")
        self.changeover = 0
        self.acc = 0.0
        self.out = 0
        self.run_min = self.avail_min = 0
        self.rate_sum = 0.0
        self.segments = []  # [start, end, what]
        self.start = minutes(self.sp["start_at"])

    def job(self, done):
        while self.entries:
            o, q = self.entries[0]
            if done[o["id"]] >= o["qty"] or q == 0:
                self.entries.pop(0)
                continue
            return self.entries[0]
        return None

    def mark(self, t, what):
        if self.segments and self.segments[-1][2] == what and self.segments[-1][1] == t:
            self.segments[-1][1] = t + STEP_MIN
        else:
            self.segments.append([t, t + STEP_MIN, what])


def _rate(config, ln, product, t, ready):
    rate = ln.meta["makes"][product]
    if not ln.meta.get("needs_cell"):
        return rate, False
    mode, cell_up = ln.sp["mode"], ready is not None and t >= ready
    if mode == "MANUAL" or (mode == "MANUAL_UNTIL_R1" and not cell_up):
        return ln.meta["manual_rate"], False
    return (rate, True) if cell_up else (0, False)


def _step_line(config, state, ln, t, done, stock, ready):
    """One step of a line: what it did and the kW it drew."""
    if not in_shift(config, state, t) or t < ln.start:
        return "off", 0
    ln.avail_min += STEP_MIN
    if ln.changeover:
        ln.changeover -= STEP_MIN
        return "changeover", 50
    entry = ln.job(done)
    if entry is None:
        return "idle", 0
    o, q = entry
    if o["product"] != ln.product and len(ln.meta["makes"]) > 1 and ln.product is not None:
        ln.product, ln.changeover = o["product"], ln.meta.get("changeover_h", 0) * 60 - STEP_MIN
        return "changeover", 50
    ln.product = o["product"]
    rate, uses_cell = _rate(config, ln, o["product"], t, ready)
    if rate == 0:
        return f"waiting for {ln.meta['needs_cell']}", 0
    if config["products"][o["product"]]["motor"] and stock["motors"] <= 0:
        return "starved: no motors", 0
    ln.acc += rate * STEP_MIN / 60
    whole = math.floor(ln.acc + 1e-9)
    left = o["qty"] - done[o["id"]]
    make = min(whole, left, q if q is not None else left)
    if config["products"][o["product"]]["motor"]:
        make = min(make, stock["motors"])
        stock["motors"] -= make
    ln.acc -= whole
    done[o["id"]] += make
    ln.out += make
    if q is not None:
        entry[1] = q - make
    ln.run_min += STEP_MIN
    ln.rate_sum += rate * STEP_MIN
    kw = ln.meta["kw"] + (config["cell"]["kw"] if uses_cell else 0)
    return o["id"] + (" (R1)" if uses_cell else (" (manual)" if ln.meta.get("needs_cell") else "")), kw


def project(config, state) -> dict:
    """What the current setpoints produce over the horizon."""
    lines = [_Line(config, state, m) for m in config["lines"]]
    done = {o["id"]: 0 for o in state["orders"]}
    finished = {}
    stock = {"motors": state["motors_on_hand"]}
    ready = cell_ready_at(config, state)
    peak, peak_at, kwh = 0, 0, 0.0
    for t in range(0, config["horizon_h"] * 60, STEP_MIN):
        kw = config["power"]["base_kw"]
        for ln in lines:
            what, k = _step_line(config, state, ln, t, done, stock, ready)
            kw += k
            ln.mark(t, what)
        for oid, n in done.items():
            if oid not in finished and n >= order(state, oid)["qty"]:
                finished[oid] = t + STEP_MIN
        if kw > peak:
            peak, peak_at = kw, t
        kwh += kw * STEP_MIN / 60
    return _summary(config, state, lines, done, finished, peak, peak_at, kwh, stock, ready)


def _oee(config, ln) -> dict:
    avail = ln.avail_min or 1
    a = ln.run_min / avail
    nameplate = max(ln.meta["makes"].values())
    p = (ln.rate_sum / ln.run_min / nameplate) if ln.run_min else 0.0
    q = 0.98
    return {"availability": round(a, 3), "performance": round(p, 3), "quality": q, "oee": round(a * p * q, 3)}


def _summary(config, state, lines, done, finished, peak, peak_at, kwh, stock, ready):
    orders = []
    late_usd = 0
    for o in state["orders"]:
        at = finished.get(o["id"])
        on_time = at is not None and at <= minutes(o["due"])
        if not on_time:
            late_usd += o["penalty_usd"]
        orders.append({"id": o["id"], "customer": o["customer"], "product": o["product"], "qty": o["qty"],
                       "due": o["due"], "made": done[o["id"]], "done_at": label(at) if at is not None else None,
                       "on_time": on_time})
    nights = sum(1 for v in state["overtime"].values() if v)
    overtime_usd = nights * config["overtime"]["usd_per_night"]
    energy_usd = round(kwh * config["power"]["usd_per_kwh"])
    per_line = {ln.meta["id"]: {"output": ln.out, **_oee(config, ln),
                                "segments": [[label(a), label(b), w] for a, b, w in ln.segments if w != "off"]}
                for ln in lines}
    plant_oee = round(sum(v["oee"] for v in per_line.values()) / len(per_line), 3)
    return {"orders": orders, "lines": per_line, "oee": plant_oee,
            "throughput": sum(v["output"] for v in per_line.values()),
            "on_time": sum(1 for o in orders if o["on_time"]), "orders_total": len(orders),
            "peak_kw": peak, "peak_at": label(peak_at), "cap_kw": config["power"]["cap_kw"],
            "motors_left": stock["motors"], "cell_ready_at": label(ready) if ready is not None else None,
            "cost_usd": {"late_penalties": late_usd, "overtime": overtime_usd, "energy": energy_usd,
                         "total": late_usd + overtime_usd + energy_usd}}
