#!/usr/bin/env bash
# Runs each integration-test binary on its own and fails if it leaves databases behind: the
# number of branch_%, snap_% and arena_test% (metadata stores) must be the same before and
# after. Run from arena/core with the compose Postgres (container deploy-pg-1) up.
set -u
count() {
  docker exec deploy-pg-1 psql -U arena -d postgres -Atc \
    "select count(*) filter (where datname like 'branch\_%') || '/' || count(*) filter (where datname like 'snap\_%') || '/' || count(*) filter (where datname like 'arena\_test%') from pg_database"
}
status=0
for f in crates/*/tests/*.rs; do
  t=$(basename "$f" .rs)
  before=$(count)
  cargo test --test "$t" >/dev/null 2>&1
  after=$(count)
  if [ "$before" = "$after" ]; then echo "ok   $t  (branch/snap/meta = $before)"; else echo "LEAK $t  before=$before after=$after"; status=1; fi
done
exit $status
