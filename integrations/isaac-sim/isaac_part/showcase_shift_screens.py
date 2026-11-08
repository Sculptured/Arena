"""The shift's boards in the showcase, painted to PNG from the plant's own state and its signage (what links from
the utility, the OT network and the hub set): the energy board (load against the utility's target), the IT/OT
gateway's screen, the dock's arrivals board and robot cell R1's HMI. Plain Python and Pillow, like showcase_hmi."""
import hashlib
from dataclasses import dataclass
from pathlib import Path

from .showcase_hmi import AMBER, DIM, GREEN, GREY, INK, RED, SCREEN, _font


@dataclass(frozen=True)
class Board:
    """What one board shows: a banner (text, colour), rows of (label, value, colour or None) and an optional bar
    (fraction filled, colour, marker fraction or None)."""
    banner: str
    color: tuple
    rows: tuple
    bar: tuple | None = None


def _signage(signage, key) -> tuple:
    s = signage.get(key) or {}
    return s.get("value"), s.get("number")


def energy(power, signage) -> Board:
    kw = (power or {}).get("kw") or 0.0
    state, target = _signage(signage, "dr")
    open_event = state in ("notified", "active") and target
    over = bool(open_event and kw > target)
    if state == "active":
        banner, color = "DEMAND RESPONSE ACTIVE", RED if over else GREEN
    elif state == "notified":
        banner, color = "DEMAND RESPONSE NOTICE", AMBER
    else:
        banner, color = "PLANT 2 ENERGY", GREY if state is None else GREEN
    rows = (("LOAD kW", f"{kw:,.0f}", RED if over else None),
            ("TARGET kW", f"{target:,.0f}" if open_event else "--", None),
            ("UTILITY", "Lakeshore P&L (simulated)", None))
    scale = max(kw, target or 0.0, 1.0) * 1.15
    bar = (kw / scale, RED if over else GREEN, (target / scale) if open_event else None)
    return Board(banner, color, rows, bar)


OT_WORDS = {None: ("REMOTE ACCESS NORMAL", GREEN), "normal": ("REMOTE ACCESS NORMAL", GREEN),
            "intrusion": ("UNKNOWN SESSION - CELL R1", RED), "isolated": ("CELL R1 ZONE ISOLATED", AMBER),
            "revoked": ("SESSION REVOKED", AMBER)}


def gateway(signage) -> Board:
    state, _ = _signage(signage, "ot")
    banner, color = OT_WORDS.get(state, (str(state).upper(), GREY))
    rows = (("GATEWAY", "GW-1 IT/OT", None), ("ZONE CELL-R1", "ISOLATED" if state in ("isolated", "revoked") else
            "CONNECTED", None), ("NETWORK", "simulated", None))
    return Board(banner, color, rows)


def dock(signage) -> Board:
    delivery, hours = _signage(signage, "truck")
    if delivery:
        return Board("INBOUND DELAYED", RED, (("TRUCK", delivery, None), ("MATERIAL", "DP780 coil, Line B", None),
                                              ("DELAY", f"+{hours:g} h" if hours else "late", RED)))
    return Board("INBOUND ON TIME", GREEN, (("TRUCK", "INB-0310-1", None), ("MATERIAL", "DP780 coil, Line B", None),
                                            ("ETA", "08:30", None)))


def cell(cell_view, signage) -> Board:
    status = (cell_view or {}).get("status", "running")
    ot, _ = _signage(signage, "ot")
    if status != "running":
        banner, color = "R1 SAFE-STOPPED", AMBER
    elif ot == "intrusion":
        banner, color = "R1 RUNNING - REMOTE SESSION", RED
    else:
        banner, color = "R1 WELDING", GREEN
    rows = (("PARTS", f"{(cell_view or {}).get('parts', 0):,}", None), ("RATE /h", f"{(cell_view or {}).get('rate_per_hour', 0)}", None),
            ("POWER kW", f"{(cell_view or {}).get('kw', 0)}", None))
    return Board(banner, color, rows)


def paint(b: Board, size=(960, 540)):
    from PIL import Image, ImageDraw
    w, h = size
    img = Image.new("RGB", size, SCREEN)
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w, h // 4], fill=b.color)
    d.text((20, h // 16), b.banner, font=_font(h // 10), fill=(255, 255, 255))
    for i, (label, value, color) in enumerate(b.rows):
        y = h // 4 + 14 + i * (h // 6)
        d.text((20, y), label, font=_font(h // 16), fill=DIM)
        d.text((w // 3, y - 4), value, font=_font(h // 11), fill=color or INK)
    if b.bar is not None:
        _bar(d, b.bar, w, h)
    return img


def _bar(d, bar, w, h) -> None:
    fraction, color, marker = bar
    x0, x1, y0, y1 = 20, w - 20, h - h // 7, h - h // 18
    d.rectangle([x0, y0, x1, y1], fill=(40, 44, 48))
    d.rectangle([x0, y0, x0 + (x1 - x0) * min(1.0, max(0.0, fraction)), y1], fill=color)
    if marker is not None:
        mx = x0 + (x1 - x0) * min(1.0, marker)
        d.line([mx, y0 - 8, mx, y1 + 8], fill=INK, width=5)


class Boards:
    """Paints each changed board once to a folder and returns the file. Same board, same file."""

    def __init__(self, folder):
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=True)

    def file_for(self, b: Board) -> str:
        key = hashlib.sha1(repr(b).encode()).hexdigest()[:16]
        path = self.folder / f"board_{key}.png"
        if not path.exists():
            paint(b).save(path)
        return str(path.resolve())
