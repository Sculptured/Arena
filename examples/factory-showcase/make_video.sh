#!/usr/bin/env bash
# The showcase video from recorded Arena runs: metrics, UI captures, charts, narration, edit.
# Nothing here runs an agent; it reads arenad (up, on :4510) and the runs it recorded.
#
#   ./make_video.sh OUT_DIR PLANT_MP4 --design A=<branch>[,<branch>] --design B=... --design C=... \
#       [--draft "why"]
#
# Needs: Python 3 with the video toolkit's venv (video/README.md), Node with Playwright
# (NODE_PATH may point at a node_modules that has it), ffmpeg, and the Kokoro environment.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${1:?OUT_DIR}"; plant="${2:?PLANT_MP4}"; shift 2
designs=(); draft=(); erp_capture=""
while [ $# -gt 0 ]; do
  case "$1" in
    --design) designs+=(--design "$2"); shift 2 ;;
    --draft) draft=(--draft "$2"); shift 2 ;;
    --erpnext-capture) erp_capture="$2"; shift 2 ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
done
PY="${SHOWCASE_PY:-$here/video/.venv/bin/python}"
mkdir -p "$out/charts" "$out/ui"
if [ -n "$erp_capture" ]; then cp "$erp_capture" "$out/ui/erpnext-records.webm"; fi
ln -sf "$(realpath "$plant")" "$out/plant.mp4"

python3 "$here/export_metrics.py" "${designs[@]}" --out "$out/metrics.json"
python3 "$here/capture/shots.py" "$out/metrics.json" > "$out/shots.json"
node "$here/capture/capture.mjs" "$out/shots.json" "$out/ui"
(cd "$here/video" && "$PY" showcase.py "$out/metrics.json" "$out" "$out/edit" "${draft[@]}")
(cd "$here/video" && nice "$PY" narrate.py "$out/edit/script.yaml" --out "$out/edit/narration")
(cd "$here/video" && nice "$PY" build.py "$out/edit/edit.yaml")
echo "video: $out/edit/showcase.mp4"
