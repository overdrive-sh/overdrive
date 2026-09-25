#!/usr/bin/env bash
# Run the Quint simulator (quint run) over every topology instance. Inside Lima.
# usage: scripts/quint-sim.sh <samples> <steps> [seed]
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
SAMPLES="${1:-2000}"; STEPS="${2:-40}"; SEED="${3:-}"
WIT_ARGS="wEpoch1 wEpoch2 wGrew wShrank wViewChanged wRetired wBootstrapped wForkedViews wCommitted3"
cd "$INC"
for M in vr_sc_4v vr_sc_3v1l vr_sc_solo; do
  echo "=== quint run --main=$M --invariant=${INV:-safety} --max-samples=$SAMPLES --max-steps=$STEPS ${SEED:+--seed=$SEED}"
  start=$(date +%s.%N)
  quint run spec/vr_sc.qnt --main="$M" --invariant="${INV:-safety}" --max-samples="$SAMPLES" --max-steps="$STEPS" \
    --witnesses $WIT_ARGS ${SEED:+--seed=$SEED} --backend=rust --mbt --out-itf="out/sim-$M-${INV:-safety}.itf.json" --verbosity=1
  rc=$?
  end=$(date +%s.%N)
  echo "=== exit=$rc wall=$(echo "$end - $start" | bc)s"
done
