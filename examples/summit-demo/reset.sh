#!/usr/bin/env bash
# One command to the demo's starting point: the demo arenad restarted fresh (detached, log in
# $ARENA_DEMO_LOG), the rehearsal runs deleted (the batch suites stay), a reference run of the world
# recorded for the World beat, and the URLs the runbook opens printed.
# Never run it while tests run. Then start the driver: python3 drive.py watch
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/env.sh"
pkill -f "python3 drive.py watch" 2>/dev/null || true
"$HERE/serve.sh"
cd "$HERE"
nice python3 drive.py reset
nice python3 drive.py world
