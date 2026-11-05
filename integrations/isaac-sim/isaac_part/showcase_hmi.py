"""The small screens on the showcase's press lines: an andon board over the line and an operator's HMI, painted to
PNG from the plant's own readings (a texture on a screen-sized panel). Plain Python and Pillow."""
import hashlib
from dataclasses import dataclass
from pathlib import Path

GREEN, AMBER, RED, GREY = (34, 170, 70), (235, 150, 20), (215, 35, 30), (90, 95, 100)
INK, DIM, SCREEN = (235, 238, 240), (150, 156, 162), (14, 17, 20)
TRIP_MM_S = 11.2
FONT = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"


@dataclass(frozen=True)
class Reading:
    """What a screen shows, as the plant reports it. `vibration` None: not meaningful yet (the drive is spinning up)."""
    line: str
    machine: str
    status: str
    spm: float
    vibration: float | None
    temp_c: float | None
    done: int
    target: int
    item: str


def status_color(status: str):
    return {"running": GREEN, "degraded": AMBER, "tripped": RED, "locked-out": RED, "down": RED,
            "maintenance": AMBER}.get(status, GREY)


def status_word(status: str) -> str:
    return {"running": "RUN", "degraded": "RUN - CHECK", "tripped": "DOWN - TRIP", "locked-out": "LOCKED OUT",
            "maintenance": "MAINTENANCE", "stopped": "STOPPED"}.get(status, status.upper())


def _font(size):
    from PIL import ImageFont
    try:
        return ImageFont.truetype(FONT, size)
    except OSError:
        return ImageFont.load_default(size)  # Pillow 10.1 and later scale the built-in font


def _rows(r: Reading) -> list:
    vib = "--" if r.vibration is None else f"{r.vibration:.1f}"
    return [("SPM", f"{r.spm:.0f}"), ("VIB mm/s", vib), ("COUNT", f"{r.done:,}"), ("TARGET", f"{r.target:,}" if r.target else "--")]


def paint(r: Reading, size=(640, 360)):
    """The screen as an image: a status banner, four figures, and a vibration bar marked at the trip level."""
    from PIL import Image, ImageDraw
    w, h = size
    img = Image.new("RGB", size, SCREEN)
    d = ImageDraw.Draw(img)
    color = status_color(r.status)
    d.rectangle([0, 0, w, h // 4], fill=color)
    d.text((16, h // 16), f"{r.machine}  {status_word(r.status)}", font=_font(h // 8), fill=(255, 255, 255))
    for i, (label, value) in enumerate(_rows(r)):
        x, y = 16 + (i % 2) * w // 2, h // 4 + 10 + (i // 2) * h // 4
        d.text((x, y), label, font=_font(h // 15), fill=DIM)
        d.text((x, y + h // 13), value, font=_font(h // 9), fill=INK)
    _vibration_bar(d, r, w, h)
    return img


def _vibration_bar(d, r: Reading, w: int, h: int) -> None:
    x0, x1, y0, y1 = 16, w - 16, h - h // 9, h - h // 22
    d.rectangle([x0, y0, x1, y1], fill=(40, 44, 48))
    trip_x = x0 + (x1 - x0) * 0.8
    if r.vibration is not None:
        f = min(1.0, r.vibration / TRIP_MM_S * 0.8)
        d.rectangle([x0, y0, x0 + (x1 - x0) * f, y1], fill=GREEN if r.vibration < 4.5 else AMBER if r.vibration < 7.1 else RED)
    d.line([trip_x, y0 - 4, trip_x, y1 + 4], fill=INK, width=3)


class Screens:
    """Paints each changed reading once to a folder and returns the file. Same reading, same file."""

    def __init__(self, folder):
        self.folder = Path(folder)
        self.folder.mkdir(parents=True, exist_ok=True)

    def file_for(self, r: Reading) -> str:
        key = hashlib.sha1(repr(r).encode()).hexdigest()[:16]
        path = self.folder / f"screen_{key}.png"
        if not path.exists():
            paint(r).save(path)
        return str(path.resolve())
