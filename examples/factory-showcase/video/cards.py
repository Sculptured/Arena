"""Generated title cards and image canvases (Pillow)."""
from PIL import Image, ImageDraw, ImageFont

from common import ACCENT, BG, FG, FONT_FILES, H, MUTED, W

S = 2  # canvases are drawn at 2x so slow zooms stay sharp


def font(weight, size):
    return ImageFont.truetype(str(FONT_FILES / f"Inter-{weight}.otf"), size * S)


def gradient():
    top, bot = (22, 30, 44), (9, 12, 18)
    img = Image.new("RGB", (W * S, H * S))
    d = ImageDraw.Draw(img)
    for y in range(H * S):
        t = y / (H * S)
        d.line([(0, y), (W * S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bot)))
    return img


def title_card(title, subtitle, out):
    img = gradient()
    d = ImageDraw.Draw(img)
    x, y = 160 * S, 400 * S
    d.rectangle([x, y, x + 10 * S, y + 190 * S], fill=ACCENT)
    d.text((x + 50 * S, y - 4 * S), title, font=font("Bold", 92), fill=FG)
    if subtitle:
        d.text((x + 50 * S, y + 125 * S), subtitle, font=font("Regular", 42), fill=MUTED)
    img.save(out)
    return out


MONO = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"


def code_card(title, lines, out):
    """A title over a block of commands, one per line (a line starting with # is a comment)."""
    img = gradient()
    d = ImageDraw.Draw(img)
    x, y = 140 * S, 120 * S
    d.rectangle([x, y, x + 10 * S, y + 90 * S], fill=ACCENT)
    d.text((x + 40 * S, y - 2 * S), title, font=font("Bold", 64), fill=FG)
    mono = ImageFont.truetype(MONO, 30 * S)
    box_y = y + 150 * S
    d.rounded_rectangle([x, box_y, (W - 140) * S, box_y + (len(lines) * 48 + 60) * S],
                        radius=14 * S, fill=(13, 17, 23), outline=(48, 54, 61), width=2 * S)
    for i, line in enumerate(lines):
        colour = MUTED if line.lstrip().startswith("#") else FG
        d.text((x + 40 * S, box_y + (30 + i * 48) * S), line, font=mono, fill=colour)
    img.save(out)
    return out


def image_canvas(src, out, margin=60):
    """Fit a screenshot inside the frame on the dark backdrop with a thin border."""
    shot = Image.open(src).convert("RGB")
    box = ((W - 2 * margin) * S, (H - 2 * margin) * S)
    k = min(box[0] / shot.width, box[1] / shot.height)
    shot = shot.resize((int(shot.width * k), int(shot.height * k)), Image.LANCZOS)
    img = gradient()
    pos = ((img.width - shot.width) // 2, (img.height - shot.height) // 2)
    d = ImageDraw.Draw(img)
    if margin:
        d.rectangle([pos[0] - 3, pos[1] - 3, pos[0] + shot.width + 3, pos[1] + shot.height + 3],
                    outline="#30363d", width=3)
    img.paste(shot, pos)
    img.save(out)
    return out
