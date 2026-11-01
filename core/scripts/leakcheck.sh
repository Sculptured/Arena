#!/usr/bin/env bash
# Runs each integration-test binary on its own and fails if it leaves databases behind: the count of
# each kind must be the same before and after. Run from arena/core with the compose Postgres
# (container deploy-pg-1) up.
#
# Postgres (the `postgres` integration, integrations/postgres/src/lib.rs): instances `pi_%`, saves
# `ps_%`; databases of runs recorded before integrations `branch_%` and `snap_%`; test metadata
# stores `arena_test%`.
# MariaDB (the `mariadb` integration, integrations/mariadb/src): instances `mi_%`, scratch views
# `mx_%`; counted too when LEAKCHECK_MARIADB names a server container of the integration (its root
# password in ARENA_MARIADB_PASSWORD, default `arena`). Never point it at a server you didn't start
# for this.
set -u
PG_KINDS=(pi ps branch snap)
MARIADB_KINDS=(mi mx)

# `a/b/c`: the count of databases named `<kind>_%` for each kind, from the SQL `query` runner.
counts() {
  local runner=$1 sql=""
  shift
  for k in "$@"; do
    sql+="${sql:+ || '/' || }sum(case when $(name_column "$runner") like '${k}\\_%' then 1 else 0 end)"
  done
  "$runner" "select $sql from $(db_table "$runner")"
}

name_column() { [ "$1" = pg ] && echo datname || echo schema_name; }
db_table() { [ "$1" = pg ] && echo pg_database || echo information_schema.schemata; }

pg() {
  docker exec deploy-pg-1 psql -U arena -d postgres -Atc "$1"
}

mariadb() {
  docker exec -e MYSQL_PWD="${ARENA_MARIADB_PASSWORD:-arena}" "$LEAKCHECK_MARIADB" \
    mariadb -uroot -N -B -e "$1"
}

count() {
  local out
  out="$(counts pg "${PG_KINDS[@]}")/$(pg "select count(*) from pg_database where datname like 'arena\_test%'")"
  if [ -n "${LEAKCHECK_MARIADB:-}" ]; then
    out+=" $(counts mariadb "${MARIADB_KINDS[@]}" | tr -d ' ')"
  fi
  echo "$out"
}

label="$(IFS=/; echo "${PG_KINDS[*]}")/meta${LEAKCHECK_MARIADB:+ $(IFS=/; echo "${MARIADB_KINDS[*]}")}"
status=0
for f in crates/*/tests/*.rs; do
  t=$(basename "$f" .rs)
  before=$(count)
  cargo test --test "$t" >/dev/null 2>&1
  after=$(count)
  if [ "$before" = "$after" ]; then echo "ok   $t  ($label = $before)"; else echo "LEAK $t  ($label) before=$before after=$after"; status=1; fi
done
exit $status
