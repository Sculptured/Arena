# The summit demo's environment: the feature tour's demo stack (arenad on :4513, Postgres
# arena-demo-pg on 5593, ERPNext arena_demo_erpnext, Twenty arena_demo_twenty, saves in
# saves-demo, target-demo) plus the UI build arenad serves.
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
source "$REPO/examples/feature-tour/env.sh"
export CARGO_INCREMENTAL=0
export ARENA_UI_DIST="$REPO/ui/dist"
export ARENA_DEMO_LOG=/home/alexsky2/sculptured-work-local/arenad-demo.log
export ARENA_DEMO_URL="http://127.0.0.1:$ARENA_PORT"
# Two 30-run batch suites on top of the feature tour's recordings (about 0.3 GiB of saves per run).
export ARENA_MAX_DB_BYTES=$((28 * 1024 * 1024 * 1024))
# Sculptured's approvals need the standalone build; the one in ~/.local/bin is too old.
if [ ! -x "$ARENA_SCULPTURED_BINARY" ]; then
  echo "missing $ARENA_SCULPTURED_BINARY (the Sculptured build the approvals part needs)" >&2
  return 1 2>/dev/null || exit 1
fi
