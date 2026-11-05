#!/usr/bin/env python3
"""Analytics charts from a metrics JSON (format: see README.md, "metrics.json").

    uv run charts.py metrics.json out/ [--animate] [--only milestones,cost,...] [--seconds 3]

Writes out/<chart>.png and, with --animate, out/<chart>.mp4 (a reveal clip, 1920x1080, 30 fps).
A chart whose data is missing from the JSON is skipped. "sample": true in the JSON stamps
"SAMPLE — NOT RESULTS" on every chart.
"""
import argparse
import shutil
import tempfile
from pathlib import Path

import numpy as np

from chartkit import (ACCENT, FG, GRID, MUTED, colors, ease, fmt_tokens, figure, label_bar,
                      plt, setup, stagger)
from common import FPS, WORK, ffmpeg, load_json


def design_info(m):
    ds = m["designs"]
    keys = list(ds)
    return keys, colors(keys), {k: ds[k].get("label", k) for k in keys}


def cost_usd(a):
    c = a.get("cost_usd", a.get("cost"))
    return (c.get("usd") if isinstance(c, dict) else c) or 0.0


def legend(ax, keys, cols, names):
    """Legend in a row just above the plot."""
    from matplotlib.patches import Patch
    ax.legend(handles=[Patch(color=cols[k], label=names[k]) for k in keys],
              loc="lower left", bbox_to_anchor=(0, 1.02), ncol=len(keys), fontsize=20,
              columnspacing=2.5)


def milestones(m, p, sample):
    keys, cols, names = design_info(m)
    fig, ax = figure("Time to each milestone", "seconds from the start of the incident", sample)
    ms = [x["name"] for x in m["designs"][keys[0]]["milestones"]]
    h = 0.8 / len(keys)
    for j, k in enumerate(keys):
        for i, mm in enumerate(m["designs"][k]["milestones"]):
            y, q = i + (j - (len(keys) - 1) / 2) * h, stagger(p, i, len(ms))
            ax.barh(y, mm["t_s"] * q, height=h * 0.9, color=cols[k])
            label_bar(ax, mm["t_s"] * q + 2, y, f"{mm['t_s']:.0f} s", q)
    ax.set_yticks(range(len(ms)), ms)
    ax.invert_yaxis()
    ax.set_xlim(0, max(x["t_s"] for k in keys for x in m["designs"][k]["milestones"]) * 1.12)
    ax.xaxis.grid(True)
    ax.set_axisbelow(True)
    legend(ax, keys, cols, names)
    return fig


def agent_rows(m):
    keys, cols, names = design_info(m)
    return [(k, a) for k in keys for a in m["designs"][k].get("agents", [])]


def bars_per_agent(m, p, sample, title, sub, value, fmt):
    keys, cols, names = design_info(m)
    rows = agent_rows(m)
    fig, ax = figure(title, sub, sample)
    for i, (k, a) in enumerate(rows):
        q = stagger(p, i, len(rows))
        v = value(a)
        ax.barh(i, v * q, color=cols[k], height=0.7)
        label_bar(ax, v * q, i, "  " + fmt(v), q)
    ax.set_yticks(range(len(rows)), [f"{a['agent']}" for _, a in rows])
    ax.invert_yaxis()
    ax.xaxis.grid(True)
    ax.set_axisbelow(True)
    ax.set_xlim(0, max(value(a) for _, a in rows) * 1.18)
    ax.xaxis.set_major_formatter(lambda v, _: fmt(v))
    legend(ax, keys, cols, names)
    return fig


def tokens(m, p, sample):
    return bars_per_agent(m, p, sample, "Tokens per agent", "prompt + completion, as recorded",
                          lambda a: a.get("tokens", 0), fmt_tokens)


def cost(m, p, sample):
    return bars_per_agent(m, p, sample, "Cost per agent", "USD, label as recorded by the run",
                          cost_usd, lambda v: f"${v:.2f}")


def context(m, p, sample):
    keys, cols, names = design_info(m)
    fig, ax = figure("Context size per model call", "prompt tokens at each call, over the run",
                     sample)
    lim = m.get("context_limit")
    top = lim or 0
    for k in keys:
        for i, (ag, pts) in enumerate(m["designs"][k].get("context", {}).items()):
            top = max(top, *[c["prompt_tokens"] for c in pts])
            draw_context_line(ax, pts, cols[k], p, f"{k}: {ag}", i)
    if lim:
        ax.axhline(lim, color="#ff7b72", lw=2.5, ls="--")
        ax.text(0.995, lim, f"model limit {fmt_tokens(lim)}", color="#ff7b72", fontsize=20,
                va="bottom", ha="right", transform=ax.get_yaxis_transform())
    ax.set_ylim(0, top * 1.12)
    ax.set_xlabel("seconds into the run")
    ax.yaxis.grid(True)
    ax.yaxis.set_major_formatter(lambda v, _: fmt_tokens(v))
    ax.legend(loc="lower left", bbox_to_anchor=(0, 1.02), fontsize=18, ncol=3)
    return fig


def draw_context_line(ax, pts, color, p, label, i):
    t = np.array([c["t_s"] for c in pts], float)
    y = np.array([c["prompt_tokens"] for c in pts], float)
    cut = t.min() + (t.max() - t.min()) * ease(p)
    keep = t <= cut
    ls = ["-", "--", ":"][i % 3]
    ax.plot(t[keep], y[keep], color=color, lw=3, ls=ls, marker="o", ms=5, label=label)


def message_agents(m, have):
    return sorted({x[e] for k in have for x in m["designs"][k]["messages"] for e in ("from", "to")})


def messages(m, p, sample):
    keys, cols, names = design_info(m)
    fig, _ = figure("Messages between agents",
                    "count per design: rows send, columns receive", sample)
    fig.axes[0].remove()
    have = [k for k in keys if m["designs"][k].get("messages")]
    agents, w = message_agents(m, have), 0.86 / len(have)
    top = max(x["count"] for k in have for x in m["designs"][k]["messages"])
    for n, k in enumerate(have):
        ax = fig.add_axes([0.14 + n * w, 0.2, w - 0.08, 0.52])
        draw_matrix(ax, m["designs"][k]["messages"], agents, cols[k], p, names[k], top, n == 0)
    return fig


def draw_matrix(ax, msgs, agents, color, p, title, top, first):
    grid = np.zeros((len(agents), len(agents)))
    for x in msgs:
        grid[agents.index(x["from"]), agents.index(x["to"])] += x["count"]
    ax.imshow(grid * ease(p), cmap=ramp(color), vmin=0, vmax=top)
    ax.set_xticks(range(len(agents)), agents, rotation=40, ha="right", fontsize=17)
    ax.set_yticks(range(len(agents)), agents if first else [], fontsize=17)
    ax.set_title(title, color=FG, fontsize=24, loc="left", pad=14)
    for s in ax.spines.values():
        s.set_visible(False)
    for (i, j), v in np.ndenumerate(grid):
        if v and p > 0.8:
            ax.text(j, i, int(v), ha="center", va="center", color="white", fontsize=20)


def ramp(color):
    from matplotlib.colors import LinearSegmentedColormap
    return LinearSegmentedColormap.from_list("r", ["#161b22", color])


def tool_calls(m, p, sample):
    keys, cols, names = design_info(m)
    systems = list(m["designs"][keys[0]]["tool_calls"])
    fig, ax = figure("Tool calls per system", "calls the agents made to each tool", sample)
    w = 0.8 / len(keys)
    for j, k in enumerate(keys):
        for i, s in enumerate(systems):
            v, q = m["designs"][k]["tool_calls"].get(s, 0), stagger(p, i, len(systems))
            x = i + (j - (len(keys) - 1) / 2) * w
            ax.bar(x, v * q, width=w * 0.9, color=cols[k])
            label_bar(ax, x, v * q + 1, str(v), q, ha="center")
    ax.set_xticks(range(len(systems)), systems)
    ax.yaxis.grid(True)
    ax.set_axisbelow(True)
    legend(ax, keys, cols, names)
    return fig


def checks(m, p, sample):
    keys, cols, names = design_info(m)
    fig, ax = figure("Checks passed", "of the scored checks for the incident", sample)
    for i, k in enumerate(keys):
        c, q = m["designs"][k]["checks"], stagger(p, i, len(keys))
        ax.barh(i, c["total"], color="#161b22", height=0.6)
        ax.barh(i, c["passed"] * q, color=cols[k], height=0.6)
        label_bar(ax, c["total"] + 0.15, i, f"{c['passed']} of {c['total']}", q)
    ax.set_yticks(range(len(keys)), [names[k] for k in keys])
    ax.invert_yaxis()
    ax.set_xlim(0, max(m["designs"][k]["checks"]["total"] for k in keys) * 1.25)
    ax.xaxis.set_visible(False)
    ax.spines["bottom"].set_visible(False)
    return fig


CHARTS = {"milestones": milestones, "tokens": tokens, "cost": cost, "context": context,
          "messages": messages, "tool_calls": tool_calls, "checks": checks}
NEEDS = {"milestones": "milestones", "tokens": "agents", "cost": "agents",
         "context": "context", "messages": "messages", "tool_calls": "tool_calls",
         "checks": "checks"}


def available(m, name):
    return all(m["designs"][k].get(NEEDS[name]) for k in m["designs"]) or (
        name in ("context", "messages") and any(
            m["designs"][k].get(NEEDS[name]) for k in m["designs"]))


def render_png(m, name, out):
    fig = CHARTS[name](m, 1.0, m.get("sample", False))
    fig.savefig(out / f"{name}.png", facecolor=fig.get_facecolor())
    plt.close(fig)


def render_clip(m, name, out, seconds, hold=1.5):
    n, tmp = int(seconds * FPS), Path(tempfile.mkdtemp(dir=WORK))
    try:
        for i in range(n + int(hold * FPS)):
            fig = CHARTS[name](m, min(i / n, 1.0), m.get("sample", False))
            fig.savefig(tmp / f"f{i:04d}.png", facecolor=fig.get_facecolor())
            plt.close(fig)
        ffmpeg("-framerate", FPS, "-i", tmp / "f%04d.png", "-c:v", "libx264", "-pix_fmt",
               "yuv420p", "-crf", "18", out / f"{name}.mp4")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("metrics")
    ap.add_argument("out")
    ap.add_argument("--animate", action="store_true")
    ap.add_argument("--only", help="comma list of: " + ", ".join(CHARTS))
    ap.add_argument("--seconds", type=float, default=3.0, help="length of the reveal")
    a = ap.parse_args()
    m, out = load_json(a.metrics), Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    WORK.mkdir(parents=True, exist_ok=True)
    setup()
    for name in (a.only.split(",") if a.only else CHARTS):
        if not available(m, name):
            print(f"skip {name}: no data")
            continue
        render_png(m, name, out)
        if a.animate:
            render_clip(m, name, out, a.seconds)
        print(f"ok {name}")


if __name__ == "__main__":
    main()
