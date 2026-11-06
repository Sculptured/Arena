"""Writes the showcase video's narration (script.yaml) and edit (edit.yaml) from metrics.json.

Every number the narrator says comes from metrics.json (export_metrics.py, from Arena's API), and
so does every comparison: which design passed the most checks, which was fastest, how close the
single agent came to its context limit. Nothing about the result is written in by hand, so the
same script tells whatever story the recorded runs tell.

    python showcase.py metrics.json FOOTAGE_DIR OUT_DIR [--draft "why"]

FOOTAGE_DIR holds plant.mp4 (the Isaac Sim render), ui/ (capture.mjs: runs.png, <K>-run.webm,
<K>-step.png, compare-<K>-<K>.webm, fork-panel.png) and charts/ (charts.py --animate). A missing
file drops its scene. `--draft` stamps every frame with the reason (partial runs, say).
"""
import argparse
import json
import sys
from pathlib import Path

import yaml

from story import Story

DESIGN_TEXT = {
    "A": "Design A is hierarchical. An incident commander reads the alarm, then delegates to four "
         "specialists, maintenance, procurement, production planning and customer communications, "
         "through Arena's message tool, and decides every change itself.",
    "B": "Design B is event-driven. The same four specialists start at once with no manager. Each "
         "subscribes to topics such as press down, diagnosis ready and purchase order approved, and "
         "acts when an event arrives.",
    "C": "Design C is a single agent. One agent gets every role's instructions and every tool, so "
         "the whole incident, and every tool result, flows through one context.",
}

RECREATE = [
    "# examples/factory-showcase/README.md has the details",
    "docker compose -f deploy/compose.yml up -d",
    "cargo build -p arena-factory -p arena-erpnext -p arena-twenty-crm",
    "(cd core && ARENA_BENCHMARK=factory cargo run -p arenad -- base)",
    "(cd core && ARENA_UI_DIST=../ui/dist cargo run -p arenad)",
    "# each design, then the video",
    "python designs/run.py --design A --out runs/A-1",
    "./make_video.sh out plant.mp4 --design A=<branch> ...",
]


def scene(sid, text, **edit):
    return {"id": sid, "text": text}, dict(edit, id=sid)


def opening(s: Story, f: Path) -> list:
    return [
        scene("cover", "", type="title", title="One plant failure, three agent designs",
              subtitle="Measured in Arena", duration=3.5, chapter="Cover"),
        scene("plant", "Ridgeline Components is a stamping plant. It is fictional, and so is everything "
              "in it. Early one morning the main bearing of press P R A 1 starts to fail. The plant is "
              "simulated in NVIDIA Isaac Sim and shown here as it runs: the press shakes harder, trips, "
              "and its beacon turns red.", type="clip", src=str(f / "plant.mp4"), max_speed=4,
              chapter="The failure",
              lower_third={"title": "Isaac Sim 6.2, rendered on a rented GPU",
                           "subtitle": "Simulated plant, fictional company", "at": 1.5}),
        scene("tools", "The incident crosses the company's systems: an E R P and its M E S records in "
              "E R P Next, a C R M in Twenty, an open-source alternative to Salesforce, an approvals "
              "system, and the outside suppliers, carriers and customers. Arena runs all of it as one "
              "world it can save, restore and score.", type="image", src=str(f / "ui" / "runs.png"),
              motion="zoom_in", chapter="The tools",
              lower_third={"title": "ERPNext, Twenty, approvals, plant",
                           "subtitle": "Open-source systems with made-up data"}),
    ]


def design_scenes(s: Story, f: Path) -> list:
    out = []
    for k in s.keys:
        text = f"{DESIGN_TEXT[k]} {s.design_result(k)}"
        out.append(scene(f"design-{k}", text, type="clip", src=str(f / "ui" / f"{k}-run.webm"),
                         chapter=s.label(k), max_speed=2,
                         lower_third={"title": s.label(k), "subtitle": s.lower_third(k)}))
        if (f / "ui" / f"{k}-step.png").exists():
            out.append(scene(f"step-{k}", s.step_line(k), type="image",
                             src=str(f / "ui" / f"{k}-step.png"), motion="zoom_in"))
    return out


def comparison(s: Story, f: Path) -> list:
    a, b = s.keys[0], s.keys[-1]
    return [
        scene("checks", s.verdict(), type="clip", src=str(f / "charts" / "checks.mp4"),
              chapter="Which design did better"),
        scene("milestones", s.speed_line(), type="clip", src=str(f / "charts" / "milestones.mp4")),
        scene("compare", "Arena's compare puts two runs side by side: where they first diverged, and "
              "every record that ended up different in the E R P, the C R M and the plant.", type="clip",
              src=str(f / "ui" / f"compare-{b}-{a}.webm"), max_speed=2),
        scene("context", s.context_line(), type="clip", src=str(f / "charts" / "context.mp4"),
              chapter="Context and roles"),
        scene("messages", s.roles_line(), type="clip", src=str(f / "charts" / "messages.mp4")),
        scene("cost", s.cost_line(), type="clip", src=str(f / "charts" / "cost.mp4"),
              chapter="Time and cost estimates"),
    ]


def closing(s: Story, f: Path) -> list:
    return [
        scene("fork", "When a run goes wrong, Arena can fork it at any step: restore the world as it "
              "was, change an instruction, and run again from there, so a fix is tested against the "
              "same moment.", type="image", src=str(f / "ui" / "fork-panel.png"), motion="zoom_in",
              chapter="Fork and retry"),
        scene("recreate", "To recreate all of this, the commands are in the showcase's read me: build "
              "Arena, seed the base world, start arenad, run each design, then export the metrics "
              "and build this video.", type="title", title="Recreate it", code=RECREATE,
              chapter="Recreate it"),
        scene("outro", s.outro(), type="title", title="Arena", subtitle="Stage agent systems before "
              "they meet the real world", chapter="Outro"),
    ]


def build(m: dict, footage: Path, out: Path, draft: str | None) -> tuple:
    s = Story(m)
    pairs = opening(s, footage) + design_scenes(s, footage) + comparison(s, footage) + closing(s, footage)
    pairs = [(t, e) for t, e in pairs if "src" not in e or Path(e["src"]).exists()]
    script = {"scenes": [t for t, _ in pairs if t["text"]]}
    edit = {"title": "One plant failure, three agent designs", "output": str(out / "showcase.mp4"),
            "narration": str(out / "narration" / "timings.json"), "music": None,
            "scenes": [e for _, e in pairs]}
    if draft:
        edit["watermark"] = f"DRAFT: {draft}"
    return script, edit


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("metrics")
    ap.add_argument("footage")
    ap.add_argument("out")
    ap.add_argument("--draft", help="stamp every frame: DRAFT: <why>")
    a = ap.parse_args()
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    script, edit = build(json.load(open(a.metrics)), Path(a.footage), out, a.draft)
    (out / "script.yaml").write_text(yaml.safe_dump(script, sort_keys=False, width=100))
    (out / "edit.yaml").write_text(yaml.safe_dump(edit, sort_keys=False, width=100))
    print(f"wrote {out / 'script.yaml'} ({len(script['scenes'])} narrated scenes) and edit.yaml",
          file=sys.stderr)


if __name__ == "__main__":
    main()
