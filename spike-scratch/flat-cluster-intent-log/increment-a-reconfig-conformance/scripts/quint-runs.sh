#!/usr/bin/env bash
# Execute the scripted counterexample runs with `quint test` (each run `.expect`s its violation).
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"; mkdir -p out
start=$(date +%s.%N)
quint test spec/vr_sc.qnt --main=vr_sc_4v --match='^phantom' --out-itf='out/run_{test}.itf.json' --verbosity=3
rc=$?
end=$(date +%s.%N)
echo "=== quint test exit=$rc wall=$(echo "$end - $start" | bc)s"
