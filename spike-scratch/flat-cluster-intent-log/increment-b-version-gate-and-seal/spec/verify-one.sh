#!/usr/bin/env bash
# One Apalache check via `quint verify`, with its full output kept as evidence.
# Usage: verify-one.sh <label> <main-module> <kind: invariant|temporal> <property> <max-steps> <port>
# Writes evidence/verify-<label>.txt (append-only within the run: header, raw output, footer).
set -u
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
label=$1 main=$2 kind=$3 prop=$4 steps=$5 port=$6
out="$EVIDENCE_DIR/verify-$label.txt"
cd "$SPEC_DIR"
{
  echo "# label:      $label"
  echo "# command:    quint verify --main $main --$kind $prop --max-steps $steps --server-endpoint localhost:$port gate_seal.qnt"
  echo "# spec sha256: $(sha256sum gate_seal.qnt | cut -d' ' -f1)"
  echo "# quint:      $(quint --version)"
  echo "# apalache:   0.56.1 (Quint default; $QUINT_HOME/apalache-dist-0.56.1)"
  echo "# java:       $(java -version 2>&1 | head -1)"
  echo "# uname -r:   $(uname -r)"
  echo "# started:    $(date -u +%FT%TZ)"
  echo "# ---------------------------------------------------------------- raw output"
} >>"$out"
start=$(date +%s)
quint verify --main "$main" "--$kind" "$prop" --max-steps "$steps" \
  --server-endpoint "localhost:$port" gate_seal.qnt >>"$out" 2>&1
rc=$?
end=$(date +%s)
{
  echo "# ---------------------------------------------------------------- end raw output"
  echo "# exit code:  $rc"
  echo "# wall-clock: $((end - start)) s"
  echo "# finished:   $(date -u +%FT%TZ)"
} >>"$out"
echo "$label rc=$rc wall=$((end - start))s"
