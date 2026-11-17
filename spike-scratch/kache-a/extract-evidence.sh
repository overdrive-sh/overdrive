#!/usr/bin/env bash
# Local helper: extract the evidence blocks a launcher run printed.
# Usage: extract-evidence.sh <run-output.log>   -> evidence/<run>/ (never overwrites)
set -euo pipefail
log="$1"; dest="$(cd "$(dirname "$0")" && pwd)/evidence"
awk -v dest="$dest" '
/^=====EVIDENCE-BEGIN /{run=$2; out="/tmp/ev-" run ".b64"; printf "" > out; on=1; next}
/^=====EVIDENCE-END /{on=0; close(out); print run; next}
on{print >> out}' "$log" | while read -r run; do
  [ -e "$dest/$run" ] && { echo "exists, skipping $run"; continue; }
  mkdir -p "$dest/$run"; base64 -d < "/tmp/ev-$run.b64" | tar -xz -C "$dest/$run"; echo "extracted $run"
done
