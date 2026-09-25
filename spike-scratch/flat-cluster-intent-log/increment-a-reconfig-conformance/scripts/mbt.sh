#!/usr/bin/env bash
# Run the quint-connect MBT tests (INSIDE Lima). usage: scripts/mbt.sh <nextest filter> <logname>
# MBT_VERBOSE=1 (or MBT_TRACE=1) -> quint-connect prints every step (needed for triage); KEEP_RAW=1 keeps the
# raw log (it is large; the host disk is shared, so it is deleted after condensing by default).
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"; mkdir -p out
start=$(date +%s.%N)
if [ -n "${MBT_VERBOSE:-}${MBT_TRACE:-}" ]; then export QUINT_VERBOSE=1; fi
cargo nextest run --test mbt -E "$1" --no-capture --no-fail-fast > "out/$2.raw.log" 2>&1
rc=$?
end=$(date +%s.%N)
# condense: the per-step state dumps are huge; keep step headers, divergence diffs and reports
sed 's/\x1b\[[0-9;]*m//g' "out/$2.raw.log" | grep -Ev "^   \+ [A-Za-z]|^   Next state:|^   Nondet picks:|^   Deriving" > "out/$2.log"
echo "=== nextest exit=$rc wall=$(echo "$end - $start" | bc)s" >> "out/$2.log"
[ "${KEEP_RAW:-0}" = "1" ] || rm -f "out/$2.raw.log"
echo "rc=$rc"
