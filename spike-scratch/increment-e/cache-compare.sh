#!/usr/bin/env bash
# PROBE increment-e — [D8c]: is `--cache=never` the right default for the volume
# role, and what does the choice cost?
#
# [D8c] picked `never` WITHOUT measuring it, on the one path that carries the
# workload's output. The first sweep measured never at ~1540 MiB/s and auto at
# ~416 MiB/s — a 3.7x gap in the OPPOSITE direction from the naive expectation
# that caching helps writes. That is surprising enough that one sample per mode
# is not good enough to report, so this runs N trials of each and prints EVERY
# sample rather than a mean.
#
# Everything except `--cache` is held fixed: same guest binary, same payload
# (passed on the kernel cmdline), same box, same shares, interleaved to keep
# any host-side drift from loading onto one arm.
#
# Usage: cache-compare.sh [trials]     (default 3)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-e
TRIALS="${1:-3}"
RESULTS="$OUT/cache-compare.txt"

: >"$RESULTS"
echo "=================================================================="
echo "[D8c] --cache comparison, $TRIALS interleaved trials per mode"
echo "payload held fixed; only virtiofsd --cache differs"
echo "=================================================================="

for i in $(seq 1 "$TRIALS"); do
  for cache in never auto; do
    printf 'trial %s cache=%-6s ... ' "$i" "$cache"
    log="$OUT/cc-$cache-$i.log"
    CACHE="$cache" "$HERE/run.sh" full >"$log" 2>&1
    rc=$?
    if [ "$rc" != 0 ]; then
      echo "DID NOT COMPLETE (rc=$rc)"
      echo "trial=$i cache=$cache INCOMPLETE rc=$rc" >>"$RESULTS"
      continue
    fi
    tp="$(grep -ho 'FS-THROUGHPUT.*' "$log" | head -1)"
    lat="$(grep -ho 'FS-LATENCY.*' "$log" | head -1)"
    echo "ok"
    { echo "trial=$i cache=$cache"
      echo "    $tp"
      echo "    $lat"; } >>"$RESULTS"
  done
done

echo
echo "=================================================================== "
echo "ALL SAMPLES (no averaging — read the spread)"
echo "=================================================================== "
cat "$RESULTS"

echo
echo "--- write-only MiB/s, by mode:"
for cache in never auto; do
  printf '    %-6s ' "$cache"
  grep -A1 "cache=$cache\$" "$RESULTS" | grep -o '[0-9.]* MiB/s (write only)' | tr '\n' ' '
  echo
done
echo
echo "--- mean ms/file, by mode:"
for cache in never auto; do
  printf '    %-6s ' "$cache"
  grep -A2 "cache=$cache\$" "$RESULTS" | grep -o 'mean [0-9.]* ms/file' | tr '\n' ' '
  echo
done
echo
echo "results: $RESULTS"
