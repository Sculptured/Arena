"""Where simulations run: a `SimulationBackend` takes the plant with a config applied and a list of
replication numbers, and answers each replication's raw result. Everything else (validating the
config, the budget, KPIs, confidence intervals, the record) is the part's, whatever the backend.

Today's backend is `des`, the open-source discrete-event model in `des.py`, in process.

A commercial simulator (Rockwell Arena Simulation or Emulate3D, on a licensed Windows machine) is
meant to plug in as a worker over HTTP. That adapter is not built; the contract it must meet is in
`integrations/line-sim/README.md` ("A remote simulator"). Asking for any backend but `des` is
refused at `start`.
"""
import hashlib
from abc import ABC, abstractmethod
from pathlib import Path

from arena_part import errors

from . import des


class SimulationBackend(ABC):
    """A simulator the part can run replications on."""

    name: str

    @abstractmethod
    def fingerprint(self) -> dict:
        """What decides its results (the model's code and version), for the part's fingerprint."""

    @abstractmethod
    def replicate(self, plant: dict, seed: int, reps: list) -> list:
        """One raw result per replication number in `reps`, in order. Replication r must give the
        same result for the same plant, seed and r (common random numbers across configs).

        A raw result is `{measured_s, lines: [{id, good, scrap, wip_mean, flow_mean_s,
        campaign_cycle_s, pm_s, stations: [{id, failures, share: {state: fraction}}]}]}` with the
        states `working`, `changeover`, `starved`, `blocked`, `down` and `pm`."""


class DesBackend(SimulationBackend):
    name = "des"

    def fingerprint(self) -> dict:
        source = Path(des.__file__).read_bytes()
        return {"backend": self.name, "model": hashlib.sha256(source).hexdigest()[:16]}

    def replicate(self, plant: dict, seed: int, reps: list) -> list:
        return [des.replicate(plant, seed, r) for r in reps]


BACKENDS = {"des": DesBackend}


def backend_for(config: dict) -> SimulationBackend:
    """The backend a part's config names (`backend`, default `des`)."""
    name = config.get("backend", "des")
    if name not in BACKENDS:
        raise errors.invalid(f"unknown simulation backend {name!r}: only 'des' is built; a remote "
                             "simulator's contract is in integrations/line-sim/README.md")
    return BACKENDS[name]()
