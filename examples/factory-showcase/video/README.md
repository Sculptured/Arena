# Factory showcase: video toolkit

Tools for step 6 of the showcase (narrated video). They contain no results: the real content
comes from recorded runs. Verified so far: the sample under `sample/` (made-up data, stamped
"SAMPLE — NOT RESULTS") builds a 30 s 1920x1080 MP4 on this machine (ffmpeg 6.1, Python 3.14).

## Setup

```bash
cd examples/factory-showcase/video
UV_PROJECT_ENVIRONMENT=$PWD/.venv uv sync        # the venv stays on the root disk
```

Work folder: `SHOWCASE_VIDEO_WORK` (default `/media/alexsky2/SSD1/sculptured-work/video`) holds the
voice model in `models/`, temporary segments, and outputs you point there. Voice: Piper
`en_GB-cori-high` (public domain, from LibriVox recordings; model card in `models/MODEL_CARD`):

```bash
M=$SHOWCASE_VIDEO_WORK/models; mkdir -p $M; B=https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_GB/cori/high
for f in en_GB-cori-high.onnx en_GB-cori-high.onnx.json MODEL_CARD; do curl -sL -o $M/$f $B/$f; done
```

`SHOWCASE_VIDEO_MODEL` points at another Piper `.onnx` voice. Check a voice's licence before use
(for example `ryan` and `hfc_*` are non-commercial).

## 1. Narration: `narrate.py`

```bash
.venv/bin/python narrate.py script.yaml [--out DIR]
```

```yaml
scenes:
  - {id: intro, text: "What the narrator says in this scene."}
```

Writes `DIR/<id>.wav` per scene and `DIR/timings.json` (duration, text, caption phrases with
start and end). Piper gives no word alignment, so caption timing splits the scene's duration over
its phrases by length: close, not exact.

## 2. Editor: `build.py`

```bash
.venv/bin/python build.py edit.yaml [--keep]
```

```yaml
title: Video title            # file metadata
output: /path/out.mp4
narration: timings.json       # from narrate.py; scene ids match
watermark: "SAMPLE — NOT RESULTS"   # optional, on every frame (leave out for real content)
music: null                   # a file path adds quiet music (music_volume, default 0.07)
scenes:
  - {id: a, type: title, title: "…", subtitle: "…", duration: 3, chapter: "Chapter name"}
  - {id: b, type: image, src: shot.png, motion: zoom_in|zoom_out|pan_left|pan_right|none, margin: 60}
  - {id: c, type: clip,  src: run.mp4, max_speed: 8, lower_third: {title: "…", subtitle: "…", at: 1, for: 4.5}}
```

A scene lasts as long as its narration plus 0.4 s lead and 0.6 s tail (or its `duration` when it
has no narration). A clip longer than that is sped up to fit (up to `max_speed`, then trimmed,
with a warning) and shows "Sped up Nx" on screen; a shorter one holds its last frame. Clip audio is
dropped; narration is the audio. Captions, lower-thirds, the speed note and the watermark are
burned in from one ASS file per scene. Scenes with a `chapter` become chapter markers.
No music is bundled: no free-licence track has been vetted. To add one, use a CC0 or CC-BY
track, and credit it in the video description.

## 3. Charts: `charts.py`

```bash
.venv/bin/python charts.py metrics.json out/ [--animate] [--only milestones,tokens,cost,context,messages,tool_calls,checks] [--seconds 3]
```

PNG per chart; with `--animate` also an MP4 reveal (bars grow, lines draw, matrices fade in; then
a 1.5 s hold). A chart is skipped when the JSON lacks its data.

### metrics.json

Shaped after arena's `GET /api/branches/{id}` `metrics` (`docs/reference.md`): `agents` entries are
that endpoint's `{agent, model, tokens, prompt_tokens, completion_tokens, cost}` as is. The rest
(milestones, per-call context, messages, tool calls, checks) is computed from a run's events by
whoever prepares the file; nothing here reads arenad.

```json
{
  "sample": false,                 // true stamps "SAMPLE — NOT RESULTS" on every chart
  "context_limit": 200000,         // the model's context window, drawn as a line (optional)
  "designs": {
    "A": {
      "label": "A, hierarchical",
      "checks": {"passed": 9, "total": 12},
      "milestones": [{"name": "first response", "t_s": 14}],   // same names in every design
      "agents": [{"agent": "commander", "model": "…", "tokens": 120000,
                  "cost": {"usd": 0.4, "label": "list-price estimate"}}],   // cost may be a number
      "context": {"commander": [{"t_s": 7.5, "prompt_tokens": 14000}]},    // one point per model call
      "messages": [{"from": "commander", "to": "maintenance", "count": 4}], // [] for a single agent
      "tool_calls": {"ERP": 41, "MES": 18, "CRM": 12}
    }
  }
}
```

`context[agent][i].prompt_tokens` is the `usage.prompt_tokens` of that agent's i-th `model_call`
event; `t_s` is seconds since the branch started.

## 4. Sample

`sample/run.sh` runs narration, charts and the editor on made-up data (`sample/script.yaml`,
`sample/metrics.json`, `sample/edit.yaml`, generated placeholder footage) and writes
`$SHOWCASE_VIDEO_WORK/sample/sample.mp4`.

## Known issue: the external SSD

On 2026-11-04 the exFAT SSD acknowledged writes but read them back as zeros after a short time
(model, WAVs, PNGs, MP4s written there became all-zero; `sync` did not finish). Outputs were
built with `SHOWCASE_VIDEO_WORK` on the root disk instead. Verify the SSD before relying on it:
write a file, wait a minute, and `dd iflag=direct` the first block back.
