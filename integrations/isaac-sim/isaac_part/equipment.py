"""What the arena adds to the factory example's plant, beside its Factory (which is never edited): robot cells and
the plant's electrical load. Both are configured (`cells`, `power`), so a world that names neither sees the plant as
it was. They are stepped from the plant's physics steps only, at a fixed rate, so what they give after an advance
doesn't depend on how the advances were split.

- A robot cell runs or is safe-stopped (the robot parked, its safety PLC holding). It counts the parts it makes
  from the physics steps it ran.
- The load is the base load plus each metered line's running or idle load (running scaled by the line's speed) plus
  each cell's. When it changes, the plant reports a `power` event, which a benchmark can link to a utility's meter.
- Signage: what the plant's boards show of other systems (a utility's target, the OT network's state, a late truck),
  set by a benchmark's links (`show_status`). Only the showcase's visuals read it: it is in no document and no tool's
  answer, so agents learn nothing from the plant they wouldn't from those systems.
"""
from arena_part.params import Refusal

RUNNING, SAFE = "running", "safe-stopped"


class Equipment:
    def __init__(self, cells: tuple, power, hz: int):
        self.meta = {c["id"]: c for c in cells}
        self.power, self.hz = power, hz
        self.every = max(1, hz // 10)  # the power is checked every 0.1 s of physics
        self.cells = {cid: {"status": RUNNING, "steps": 0} for cid in self.meta}
        self.kw = None  # the last load reported
        self.signage = {}  # key -> {"value", "number"}

    # ---- saved state -----------------------------------------------------------------------------------

    def snapshot(self) -> dict:
        return {"cells": self.cells, "kw": self.kw, "signage": self.signage}

    def restore(self, snap: dict) -> None:
        self.cells, self.kw, self.signage = snap["cells"], snap["kw"], snap.get("signage", {})

    # ---- the physics step ------------------------------------------------------------------------------

    def step(self, steps: int, factory) -> None:
        """After physics step number `steps`: counts the running cells' steps; every 0.1 s, the load."""
        for c in self.cells.values():
            if c["status"] == RUNNING:
                c["steps"] += 1
        if self.power is not None and steps % self.every == 0:
            self._report(factory)

    def _report(self, factory) -> None:
        kw = self.load(factory)
        if kw == self.kw:
            return
        self.kw = kw
        meter = self.power.get("meter", "Plant meter")
        factory.emit("power", meter, f"{meter}: {kw:.1f} kW", {"kw": kw, "meter": meter})

    def load(self, factory) -> float:
        lines = self.power.get("lines", {})
        total = float(self.power.get("base_kw", 0))
        for lid, kw in lines.items():
            running = factory.line_status(lid) == "running"
            total += kw["running_kw"] * factory.lines[lid]["boost"] if running else kw["idle_kw"]
        for cid, c in self.cells.items():
            total += self.meta[cid]["running_kw" if c["status"] == RUNNING else "idle_kw"]
        return round(total, 1)

    # ---- tools -----------------------------------------------------------------------------------------

    def _cell(self, cid: str) -> dict:
        if cid not in self.cells:
            raise Refusal(f"unknown cell {cid}")
        return self.cells[cid]

    def safe_stop(self, factory, cid: str, reason) -> dict:
        c = self._cell(cid)
        if c["status"] == SAFE:
            raise Refusal(f"{cid} is safe-stopped already")
        c["status"] = SAFE
        why = f" ({reason})" if reason else ""
        factory.emit("cell", cid, f"{cid} safe-stopped: robot parked, safety PLC holding{why}",
                     {"cell": cid, "status": SAFE})
        return self.cell_view(cid)

    def start(self, factory, cid: str) -> dict:
        c = self._cell(cid)
        if c["status"] == RUNNING:
            raise Refusal(f"{cid} is running already")
        c["status"] = RUNNING
        factory.emit("cell", cid, f"{cid} running", {"cell": cid, "status": RUNNING})
        return self.cell_view(cid)

    def show_status(self, key: str, value: str, number) -> dict:
        """What a board shows for `key` from now on (a link sets it)."""
        if not key:
            raise Refusal("key must not be empty")
        self.signage[key] = {"value": value, "number": number}
        return {"key": key, **self.signage[key]}

    # ---- views -----------------------------------------------------------------------------------------

    def cell_view(self, cid: str) -> dict:
        meta, c = self.meta[cid], self.cells[cid]
        parts = int(c["steps"] * meta["rate_per_hour"] // (self.hz * 3600))
        kw = meta["running_kw" if c["status"] == RUNNING else "idle_kw"]
        return {"id": cid, "name": meta["name"], "line": meta.get("line"), "zone": meta.get("zone"),
                "robot": meta.get("robot"), "status": c["status"], "safe_stopped": c["status"] != RUNNING,
                "parts": parts, "rate_per_hour": meta["rate_per_hour"], "kw": kw}

    def visual_state(self, factory) -> dict:
        """What the showcase's visuals read each frame: the cells, the load and the signage."""
        power = None if self.power is None else {"kw": self.load(factory)}
        return {"cells": {cid: self.cell_view(cid) for cid in self.cells}, "power": power, "signage": self.signage}

    def state(self, factory) -> dict:
        """What Plant.state adds to the factory's state: `cells`, and `power` when it is configured."""
        out = {"cells": [self.cell_view(cid) for cid in self.cells]}
        if self.power is not None:
            out["power"] = {"meter": self.power.get("meter", "Plant meter"), "kw": self.load(factory),
                            "reported_kw": self.kw}
        return out


def check_config(cells, power) -> None:
    """The shape of `cells` and `power`; ValueError, which the config answers with -32602."""
    if not isinstance(cells, list):
        raise ValueError("cells must be a list")
    for c in cells:
        need = ("id", "name", "rate_per_hour", "running_kw", "idle_kw")
        if not isinstance(c, dict) or any(k not in c for k in need):
            raise ValueError(f"each cell needs {', '.join(need)}")
    if power is not None:
        lines = power.get("lines", {}) if isinstance(power, dict) else None
        if not isinstance(lines, dict) or not all({"running_kw", "idle_kw"} <= set(v) for v in lines.values()):
            raise ValueError("power needs lines, each with running_kw and idle_kw")
