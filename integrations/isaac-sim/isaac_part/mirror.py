"""Mirror mode (package isaac-sim-mirror): a running factory's events and state, read over its HTTP API. It only
ever GETs, so it never changes the plant. A live plant can't be rewound, so the mirror has no restore."""
import json
import urllib.parse
import urllib.request
from dataclasses import dataclass

from arena_part import errors

from . import views
from .base import DocsPart

NAME = "isaac-sim-mirror"
DEFAULT_URL = "http://127.0.0.1:8090"
TIMEOUT_S = 10
PAGE = 500  # events per GET /events


@dataclass
class Link:
    url: str
    last_seq: int  # the last plant event seen


def _get(url: str, path: str, query: dict | None = None):
    full = url + path + ("?" + urllib.parse.urlencode(query) if query else "")
    try:
        with urllib.request.urlopen(full, timeout=TIMEOUT_S) as r:
            return json.loads(r.read())
    except (OSError, ValueError) as e:
        raise errors.internal(f"the plant at {url} did not answer GET {path}: {e}") from None


def _plant_url(config) -> str:
    url = (config or {}).get("plant_url") or DEFAULT_URL
    if not isinstance(url, str) or urllib.parse.urlsplit(url).scheme not in ("http", "https"):
        raise errors.invalid(f"plant_url must be an http URL, got {url!r}")
    return url.rstrip("/")


class Mirror(DocsPart):
    name = NAME

    def describe(self):
        return {"name": NAME, "version": "0.1.0", "protocol": 1, "restore": {"exact": "none", "approximate": False},
                "uses_clock": True, "fingerprint": {"adapter": 1, "mode": "mirror"}, "tools": []}

    def start(self, instance, config, seed, clock_ms, from_=None):
        """Follows the plant from now: events before the start are its history, not this world's."""
        if from_ is not None:
            raise errors.cant_restore(f"{NAME} follows a live plant, which can't be rewound")
        url = _plant_url(config)
        self.instances[instance] = Link(url, int(_get(url, "/state").get("log_seq") or 0))

    def call_tool(self, instance, name, args, clock_ms):
        raise errors.invalid(f"unknown tool {name}: {NAME} has no tools")

    def advance(self, instance, clock_ms):
        link = self._instance(instance)
        found = _get(link.url, "/events", {"since": link.last_seq, "limit": PAGE})
        if found:
            link.last_seq = found[-1]["seq"]
        events = [views.arena_event(e) for e in found]
        return {"changed": bool(events), "events": events}

    def save(self, instance, save, dir):
        """The plant's /state now, kept for display; nothing can start from it."""
        return {"bytes": views.write_docs(dir, self._live_docs(instance)), "exact": False}

    def _live_docs(self, instance) -> dict:
        return views.docs_of(_get(self._instance(instance).url, "/state"))
