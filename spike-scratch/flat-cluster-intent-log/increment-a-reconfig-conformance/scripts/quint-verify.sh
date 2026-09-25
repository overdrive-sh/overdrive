#!/usr/bin/env bash
# Bounded model checking with Apalache (quint verify). Inside Lima.
# usage: scripts/quint-verify.sh <main> <invariant> <max-steps>
set -uo pipefail
INC="$(cd "$(dirname "$0")/.." && pwd)"
source "$INC/scripts/env.sh"
cd "$INC"; mkdir -p out
start=$(date +%s.%N)
quint verify spec/vr_sc.qnt --main="$1" --invariant="$2" --max-steps="$3" --verbosity=2 \
  --out-itf="out/verify-$1-$2-$3.itf.json"
rc=$?
end=$(date +%s.%N)
echo "=== quint verify --main=$1 --invariant=$2 --max-steps=$3 exit=$rc wall=$(echo "$end - $start" | bc)s"
