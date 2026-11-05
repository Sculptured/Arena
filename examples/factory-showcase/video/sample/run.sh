#!/usr/bin/env bash
# End-to-end sample: narration, charts, a generated footage clip, then the edit. Made-up data.
# usage: sample/run.sh   (set SHOWCASE_VIDEO_WORK to choose the work folder)
set -euo pipefail
cd "$(dirname "$0")/.."
W="${SHOWCASE_VIDEO_WORK:-/media/alexsky2/SSD1/sculptured-work/video}"
S="$W/sample"; mkdir -p "$S"
PY=.venv/bin/python
$PY narrate.py sample/script.yaml --out "$S/narration"
$PY charts.py sample/metrics.json "$S/charts" --animate --seconds 6 --only context
$PY charts.py sample/metrics.json "$S/charts" --only checks
nice -n 10 ffmpeg -y -hide_banner -loglevel error -f lavfi -i "color=c=#2b3340:s=1280x720:r=30:d=20" \
  -vf "drawbox=x=0:y=470:w=1280:h=60:color=#555e6b:t=fill,drawbox=x='mod(t*160,1400)-120':y=410:w=90:h=60:color=#d29922:t=fill,drawbox=x='mod(t*160+500,1400)-120':y=410:w=90:h=60:color=#d29922:t=fill,drawbox=x=880:y=150:w=140:h=250:color=#8b949e:t=fill,drawbox=x=900:y='170+30*abs(sin(t*3))':w=100:h=120:color=#f85149:t=fill,drawtext=fontfile=/usr/share/fonts/opentype/inter/Inter-Bold.otf:text='SAMPLE FOOTAGE (generated)':x=60:y=60:fontsize=44:fontcolor=white" \
  -c:v libx264 -pix_fmt yuv420p "$S/footage.mp4"
sed "s#@S@#$S#g" sample/edit.yaml > "$S/edit.yaml"
$PY build.py "$S/edit.yaml"
