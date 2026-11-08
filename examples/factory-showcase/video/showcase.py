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
    "A": "Design A is hierarchical. An incident commander reads the alarm and delegates to four "
         "specialists: maintenance, procurement, production planning, and customer communications. "
         "They coordinate through Arena's message tool. The commander decides every change.",
    "B": "Design B is event-driven. The same four specialists start at once with no manager. Each "
         "subscribes to topics such as press down, diagnosis ready, and purchase order approved. "
         "Each acts when an event arrives.",
    "C": "Design C is a single agent. It gets every role's instructions and every tool. "
         "The whole incident, including every tool result, flows through one context.",
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
        scene("cover", "This is an experimental setup for testing agent systems. "
              "The company, plant and business records are fictional. "
              "The agent runs are recorded, and their outcomes are measured in Sculptured Arena.",
              type="title", title="An experiment in agent systems",
              subtitle="Sculptured Arena · simulated business · recorded runs", chapter="Experimental setup"),
        scene("plant", "Our test case is a bearing failure at Ridgeline Components, a fictional stamping plant. "
              "The recorded tests use a deterministic plant stand-in. This Isaac Sim visualization shows the failure scenario. "
              "The question is how each agent design handles the work that follows: diagnosis, purchasing, "
              "production planning, and customer communication.", type="clip", src=str(f / "plant.mp4"), max_speed=4,
              chapter="The failure",
              lower_third={"title": "Isaac Sim 6.2, rendered on a rented GPU",
                           "subtitle": "Simulated plant, fictional company", "at": 1.5}),
        scene("tools", "Arena is the control dashboard for the experiment. "
              "It brings the agent runs, recorded actions, saved world states, and comparisons into one place. "
              "The world includes business records in ERPNext, customer records in Twenty, "
              "and the plant and approval systems.", type="image", src=str(f / "ui" / "runs.png"),
              motion="none", chapter="The control dashboard",
              lower_third={"title": "ERPNext, Twenty, approvals, plant",
                           "subtitle": "Open-source systems with made-up data"}),
        scene("erpnext", "Here is the business system behind those tool calls: ERPNext. "
              "These are seeded experimental records in the application's own interface. "
              "Start with an affected order, then open the purchase order created during the run. "
              "We can inspect the supplier, line items, quantities, and document status, "
              "then return to Arena to see which agent made the change.", type="clip",
              src=str(f / "ui" / "erpnext-records.webm"), max_speed=1,
              chapter="Inside ERPNext"),
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
        scene("checks", s.verdict(), type="clip", src=str(f / "ui" / "checks.webm"),
              chapter="Which design did better"),
        scene("milestones", s.speed_line(), type="clip", src=str(f / "ui" / "milestones.webm")),
        scene("compare", "Arena's compare puts two runs side by side: where they first diverged, and "
              "the saved records that ended up different in the parts Arena can compare.", type="clip",
              src=str(f / "ui" / f"compare-{b}-{a}.webm"), max_speed=2),
        scene("context", s.context_line(), type="clip", src=str(f / "ui" / "context.webm"),
              chapter="Context and roles"),
        scene("messages", s.roles_line(), type="clip", src=str(f / "ui" / "messages.webm")),
        scene("cost", s.cost_line(), type="clip", src=str(f / "ui" / "cost.webm"),
              chapter="Time and cost estimates"),
        scene("arena-metrics", "These measurements also belong in the run dashboard. "
              "Open the deployment estimate to inspect the measured duration and Arena's overhead. "
              "The agent table shows input tokens, output tokens, and the source of each cost figure. "
              "The bars summarize cumulative token use. They are not the context size of a single call. "
              "This is evidence for deployment planning, not a production performance guarantee.",
              type="clip", src=str(f / "ui" / "arena-metrics.webm"), max_speed=1,
              chapter="Inspect measurements in Arena"),
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
    for _, edit_scene in pairs:
        if "src" in edit_scene and Path(edit_scene["src"]).parent.name == "ui":
            edit_scene.update(safe_ui=True, motion="none", margin=0)
            # Keep labels from covering the interface; captions have their own bottom band.
            edit_scene.pop("lower_third", None)
    missing = [e["src"] for _, e in pairs if "src" in e and not Path(e["src"]).exists()]
    if missing and not draft:
        raise FileNotFoundError("Required footage is missing; capture it before a final render: " + ", ".join(missing))
    if missing:
        print("Draft omits missing footage: " + ", ".join(missing), file=sys.stderr)
    pairs = [(t, e) for t, e in pairs if "src" not in e or Path(e["src"]).exists()]
    script = {"backend": "kokoro", "speaker": "af_heart", "language": "a", "speed": 0.96,
              "pause": 0.5, "scenes": [t for t, _ in pairs if t["text"]]}
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
