# The local pharma stack: arenad on :4513 with the pharma benchmark (Postgres arena-demo-pg on 5593,
# ERPNext arena_demo_erpnext, Twenty arena_demo_twenty, saves in saves-demo, target-demo) and the UI
# build it serves. Agents are real models: Claude Code or Codex CLIs, or API providers when their
# keys are set in the environment.
export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/home/alexsky2/sculptured-work-local/target-demo
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export ARENA_BIN_DIR=$CARGO_TARGET_DIR/debug
export ARENA_SAVES=/home/alexsky2/sculptured-work-local/saves-demo
export ARENA_PG_URL=postgres://arena:arena@127.0.0.1:5593/postgres
export ARENA_BENCHMARK=pharma
export ARENA_PORT=4513
export ARENA_MAX_DB_BYTES=$((28 * 1024 * 1024 * 1024))
export ARENA_SCULPTURED_BINARY=/home/alexsky2/sculptured-work-local/sculptured-bin/sculptured
export PATH=/home/alexsky2/sculptured-work-local/sculptured-bin:$PATH
export UV_CACHE_DIR=/home/alexsky2/sculptured-work-local/uv-cache-demo
export ARENA_RUN_ERPNEXT='exec env ARENA_ERPNEXT_PROJECT=arena_demo_erpnext ARENA_ERPNEXT_DB_CONTAINER=arena-demo-erpnext-db ARENA_ERPNEXT_DB_VOLUME=arena-demo-erpnext-db ARENA_ERPNEXT_DB_PORT=3393 ARENA_ERPNEXT_PORT=8193 /home/alexsky2/sculptured-work-local/target-demo/debug/arena-erpnext'
export ARENA_RUN_TWENTY_CRM='exec env ARENA_TWENTY_PROJECT=arena_demo_twenty ARENA_TWENTY_DB_PORT=5594 /home/alexsky2/sculptured-work-local/target-demo/debug/arena-twenty-crm'
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
export CARGO_INCREMENTAL=0
export ARENA_UI_DIST="$REPO/ui/dist"
export ARENA_DEMO_LOG=/home/alexsky2/sculptured-work-local/arenad-demo.log
export ARENA_DEMO_URL="http://127.0.0.1:$ARENA_PORT"
# Sculptured's approvals need the standalone build; the one in ~/.local/bin is too old.
if [ ! -x "$ARENA_SCULPTURED_BINARY" ]; then
  echo "missing $ARENA_SCULPTURED_BINARY (the Sculptured build the approvals part needs)" >&2
  return 1 2>/dev/null || exit 1
fi
