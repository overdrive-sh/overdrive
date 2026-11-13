#!/usr/bin/env bash
# Usage: run.sh <label> <cargo args...>   (ordinary Metal grammar)
# raw log -> out/ (ignored); filtered evidence -> evidence/<label>.txt
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-validation/increment-a
label=$1; shift
cargo xtask metal run -- cargo "$@" > $D/out/$label.log 2>&1
code=$?
{ echo "# $label  cargo $*  exit: $code"; cat $D/out/$label.log; } > $D/evidence/$label.txt
echo "exit $code -> $D/evidence/$label.txt"
exit $code
