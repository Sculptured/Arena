#!/usr/bin/env bash
# Stops the demo arenad (only the one built in target-demo) and starts it again, detached, with
# its output in $ARENA_DEMO_LOG; waits until it answers. Never run it while tests run.
set -euo pipefail
source "$(dirname "$0")/env.sh"
stop_demo() {
  local pids
  pids="$(pgrep -f "^$ARENA_BIN_DIR/arenad\$" || true)"
  [ -z "$pids" ] && return 0
  kill $pids
  for _ in $(seq 1 60); do pgrep -f "^$ARENA_BIN_DIR/arenad\$" >/dev/null || return 0; sleep 0.5; done
  echo "the demo arenad did not stop" >&2; return 1
}
stop_demo
[ -d "$ARENA_UI_DIST" ] || { echo "build the UI first: (cd ui && npm run build)" >&2; exit 1; }
nice "$ARENA_BIN_DIR/arenad" base >>"$ARENA_DEMO_LOG" 2>&1
setsid nohup nice "$ARENA_BIN_DIR/arenad" >>"$ARENA_DEMO_LOG" 2>&1 < /dev/null &
for _ in $(seq 1 240); do
  if curl -fsS "$ARENA_DEMO_URL/api/health" >/dev/null 2>&1; then
    echo "arenad is up on $ARENA_DEMO_URL (log: $ARENA_DEMO_LOG)"; exit 0
  fi
  sleep 0.5
done
echo "arenad did not answer; see $ARENA_DEMO_LOG" >&2; exit 1
