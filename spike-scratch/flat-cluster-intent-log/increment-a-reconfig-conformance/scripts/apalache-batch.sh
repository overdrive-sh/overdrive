#!/usr/bin/env bash
# Bounded model checking batch (Apalache via `quint verify`), run DETACHED inside Lima. Sequential runs on a
# private Apalache server port (the sibling increment owns 18923). Each run: all 7 safety invariants,
# BMC to the given depth, a per-run wall-clock cap; the log records exit status + wall time.
# usage: scripts/apalache-batch.sh <tag> "<main>:<depth>[:random] ..."
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"
TAG="$1"; shift
OUT="evidence/apalache"; mkdir -p "$OUT"
PORT="${APALACHE_PORT:-18931}"
CAP="${APALACHE_CAP:-5400}"
INVS="agreement completeness singlePrimary promotionCaughtUp shrinkAcked learnerNoVote noFailStop"
echo "batch $TAG start $(date -Is) cap=${CAP}s port=$PORT" > "$OUT/$TAG.summary"
for job in $@; do
  M="${job%%:*}"; rest="${job#*:}"; D="${rest%%:*}"; MODE="${rest#*:}"
  [ "$MODE" = "$rest" ] && MODE=bmc
  EXTRA=""; [ "$MODE" = "random" ] && EXTRA="--random-transitions=true"
  LOG="$OUT/$TAG-$M-d$D-$MODE.log"
  start=$(date +%s)
  timeout "$CAP" quint verify "${SPEC:-spec/vr_sc.qnt}" --main="$M" --max-steps="$D" --invariants $INVS $EXTRA \
    --server-endpoint="localhost:$PORT" --verbosity=1 > "$LOG" 2>&1
  rc=$?
  end=$(date +%s)
  verdict=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -v protobuf | grep -E "No violation found|violation|Found an issue|rror" | head -3 | tr '\n' ' ')
  echo "$M depth=$D mode=$MODE rc=$rc wall=$((end - start))s :: $verdict" >> "$OUT/$TAG.summary"
done
echo "batch $TAG done $(date -Is)" >> "$OUT/$TAG.summary"
