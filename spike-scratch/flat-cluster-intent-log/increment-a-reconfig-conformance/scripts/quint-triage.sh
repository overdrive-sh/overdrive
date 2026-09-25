#!/usr/bin/env bash
# Reproduce one violating simulation seed, report which invariants fail, and dump the ITF trace.
# usage: scripts/quint-triage.sh <main-module> <seed> <samples> <steps> <tag>
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
M="$1"; SEED="$2"; SAMPLES="$3"; STEPS="$4"; TAG="$5"
cd "$INC"; mkdir -p out
quint run spec/vr_sc.qnt --main="$M" --max-samples="$SAMPLES" --max-steps="$STEPS" --seed="$SEED" \
  --backend=rust --mbt --out-itf="out/$TAG.itf.json" --verbosity=2 \
  --invariants agreement completeness singlePrimary promotionCaughtUp shrinkAcked learnerNoVote noFailStop \
  2>&1 | grep -v "^\[State\|^  \|^$\|^{\|^}"
echo "rc=${PIPESTATUS[0]}"
