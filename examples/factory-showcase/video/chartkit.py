"""Chart styling and small helpers shared by the chart functions."""
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib import font_manager

from common import ACCENT, BG, FG, FONT, FONT_FILES, H, MUTED, PALETTE, W

GRID = "#21262d"


def setup():
    for f in FONT_FILES.glob("Inter-*.otf"):
        font_manager.fontManager.addfont(str(f))
    plt.rcParams.update({
        "font.family": FONT, "font.size": 22, "text.color": FG,
        "axes.facecolor": BG, "figure.facecolor": BG, "axes.edgecolor": GRID,
        "axes.labelcolor": MUTED, "xtick.color": MUTED, "ytick.color": MUTED,
        "axes.spines.top": False, "axes.spines.right": False,
        "grid.color": GRID, "grid.linewidth": 1.2, "legend.frameon": False,
        "legend.labelcolor": FG,
    })


def figure(title, subtitle="", sample=False):
    fig = plt.figure(figsize=(W / 100, H / 100), dpi=100)
    fig.text(0.06, 0.93, title, fontsize=38, fontweight="bold", va="center")
    if subtitle:
        fig.text(0.06, 0.875, subtitle, fontsize=20, color=MUTED, va="center")
    if sample:
        fig.text(0.5, 0.5, "SAMPLE — NOT RESULTS", fontsize=96, fontweight="bold",
                 color="white", alpha=0.05, ha="center", va="center", rotation=18)
        fig.text(0.94, 0.04, "SAMPLE — NOT RESULTS", fontsize=22, fontweight="bold",
                 color="#ff7b72", ha="right", va="center")
    ax = fig.add_axes([0.17, 0.19, 0.78, 0.54])
    return fig, ax


def colors(keys):
    return {k: PALETTE[i % len(PALETTE)] for i, k in enumerate(keys)}


def ease(p):
    """Smooth 0..1 progress."""
    p = min(max(p, 0.0), 1.0)
    return 1 - (1 - p) ** 3


def stagger(p, i, n, overlap=0.5):
    """Progress of item i of n when the whole reveal is at p: items start one after another."""
    span = 1 / (1 + (n - 1) * (1 - overlap)) if n else 1
    start = i * span * (1 - overlap)
    return ease((p - start) / span)


def label_bar(ax, x, y, text, p, **kw):
    if p > 0.85:
        ax.text(x, y, text, color=FG, va="center", fontsize=20,
                alpha=min((p - 0.85) / 0.15, 1.0), **kw)


def fmt_tokens(n):
    return f"{round(n / 1000, 1):g}k" if n >= 1000 else str(int(n))
