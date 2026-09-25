#!/usr/bin/env bash
# One churn conformance round (inside Lima): typecheck, regenerate the stepChurn ITF traces per topology,
# replay them through quint-connect against real viewstamp, and report replay verdicts + coverage.
# usage: scripts/churn-round.sh <seed> <samples> <steps> <logname>
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"
quint typecheck spec/vr_sc.qnt || exit 1
for m in vr_sc_3v1l vr_sc_4v vr_sc_solo; do ./scripts/regen-traces.sh "$m" "$1" "$2" "$3" stepChurn > /dev/null; done
MBT_SAMPLES="$2" MBT_STEPS="$3" MBT_SEED="$1" ./scripts/mbt.sh "test(churn_)" "$4"
grep -E "^====|FINDING|traces=|nextest exit" "out/$4.log"
python3 scripts/coverage.py out/qc-vr_sc_4v-stepChurn out/qc-vr_sc_3v1l-stepChurn out/qc-vr_sc_solo-stepChurn
