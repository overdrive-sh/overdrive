#!/usr/bin/env bash
# Regenerate the ITF traces quint-connect replayed (same quint run parameters) into out/qc-<main>[-<step>]/.
# usage: scripts/regen-traces.sh <main> <seed> <samples> <steps> [step-action]
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
DIR="out/qc-$1${5:+-$5}"
cd "$INC"; rm -rf "$DIR"; mkdir -p "$DIR"
quint run spec/vr_sc.qnt --main="$1" --seed "$2" --max-samples "$3" --n-traces "$3" --max-steps "$4" \
  --mbt ${5:+--step "$5"} --out-itf "$DIR/run_{seq}.itf.json" --verbosity 0
echo "regen rc=$?"
