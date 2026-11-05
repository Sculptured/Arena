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
