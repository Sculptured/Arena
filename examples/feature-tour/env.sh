export CARGO_BUILD_JOBS=2
export CARGO_TARGET_DIR=/home/alexsky2/sculptured-work-local/target-demo
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_PROFILE_TEST_DEBUG=0
export ARENA_BIN_DIR=$CARGO_TARGET_DIR/debug
export ARENA_SAVES=/home/alexsky2/sculptured-work-local/saves-demo
export ARENA_PG_URL=postgres://arena:arena@127.0.0.1:5593/postgres
export ARENA_BENCHMARK=pharma
export ARENA_PORT=4513
export ARENA_SCULPTURED_BINARY=/home/alexsky2/sculptured-work-local/sculptured-bin/sculptured
export PATH=/home/alexsky2/sculptured-work-local/sculptured-bin:$PATH
export UV_CACHE_DIR=/home/alexsky2/sculptured-work-local/uv-cache-demo
export ARENA_RUN_ERPNEXT='exec env ARENA_ERPNEXT_PROJECT=arena_demo_erpnext ARENA_ERPNEXT_DB_CONTAINER=arena-demo-erpnext-db ARENA_ERPNEXT_DB_VOLUME=arena-demo-erpnext-db ARENA_ERPNEXT_DB_PORT=3393 ARENA_ERPNEXT_PORT=8193 /home/alexsky2/sculptured-work-local/target-demo/debug/arena-erpnext'
export ARENA_RUN_TWENTY_CRM='exec env ARENA_TWENTY_PROJECT=arena_demo_twenty ARENA_TWENTY_DB_PORT=5594 /home/alexsky2/sculptured-work-local/target-demo/debug/arena-twenty-crm'
# These recordings never use a model provider.
unset OPENROUTER_API_KEY OPENAI_API_KEY ANTHROPIC_API_KEY ARENA_LOCAL_MODELS
