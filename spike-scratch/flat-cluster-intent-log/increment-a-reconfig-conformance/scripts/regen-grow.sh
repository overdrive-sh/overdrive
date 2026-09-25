#!/usr/bin/env bash
# Regenerate the seeded grow+churn traces (`quint test`, one trace per seed, exactly as tests/mbt.rs
# grow_churn replays them) into out/gt-<main>/run_<i>.itf.json for triage / coverage.
# usage: scripts/regen-grow.sh <main> <test> <base-seed-hex> <samples>
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"; DIR="out/gt-$1"; rm -rf "$DIR"; mkdir -p "$DIR"
base=$((16#${3#0x}))
for ((i = 0; i < $4; i++)); do
  seed=$(printf '0x%x' $((base + i)))
  quint test spec/vr_sc.qnt --main="$1" --match="^$2\$" --seed "$seed" --max-samples 1 \
    --out-itf "$DIR/run_$i.itf.json" --verbosity 0 > /dev/null 2>&1 || echo "quint test failed for seed $seed"
done
echo "regen-grow: $(ls "$DIR" | wc -l) traces"
