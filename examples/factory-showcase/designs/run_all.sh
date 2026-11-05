#!/usr/bin/env bash
# The metric runs, one after another so that no run shares the machine with another:
# A, B, C, then again A, B, C. Usage: run_all.sh RUNS_DIR [REPEATS] [MAX_MINUTES]
set -u
out="${1:?the folder for the runs}"
repeats="${2:-2}"
minutes="${3:-60}"
cd "$(dirname "$0")"
for r in $(seq 1 "$repeats"); do
  for d in A B C; do
    dir="$out/real-$d-$r"
    mkdir -p "$dir"
    echo "$(date +%FT%T) start $d-$r" >> "$out/real.log"
    python3 run.py --design "$d" --out "$dir" --max-minutes "$minutes" > "$dir/run.out" 2>&1
    echo "$(date +%FT%T) end $d-$r: $(tail -1 "$dir/run.out" | cut -c1-300)" >> "$out/real.log"
    if grep -q '"outcome": "usage limit"' "$dir/run.out"; then
      echo "$(date +%FT%T) stopped: the subscription's usage limit" >> "$out/real.log"
      exit 1
    fi
  done
done
