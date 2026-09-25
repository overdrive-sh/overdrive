#!/usr/bin/env bash
# One conformance round (inside Lima): typecheck, scripted runs, regenerate the replayed ITF traces for
# triage, and replay every topology through quint-connect.
# usage: scripts/round.sh <seed> <samples> <steps> <logname> [extra nextest filter]
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"
quint typecheck spec/vr_sc.qnt || exit 1
./scripts/quint-runs.sh > out/runs.log 2>&1; echo "quint-runs rc=$?"
for m in vr_sc_3v1l vr_sc_4v vr_sc_solo; do ./scripts/regen-traces.sh "$m" "$1" "$2" "$3" > /dev/null; done
MBT_SAMPLES="$2" MBT_STEPS="$3" MBT_SEED="$1" ./scripts/mbt.sh "${5:-test(sim_) | test(run_phantom)}" "$4"
sed 's/\x1b\[[0-9;]*m//g' out/runs.log | grep -E "ok |failed|rror"
grep -E "^====|FINDING|traces=|nextest exit" "out/$4.log"
