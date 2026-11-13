#!/usr/bin/env bash
# Usage: run.sh <label> '<nextest filterset>'
# Runs the probe through the ordinary Metal grammar; raw log -> out/ (ignored),
# filtered evidence -> evidence/<label>.txt (committed).
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-module-guest/increment-a
mkdir -p $D/out $D/evidence
cargo xtask metal run -- cargo nextest run --manifest-path $D/Cargo.toml --no-fail-fast ${EXTRA:-} \
  --success-output immediate --failure-output immediate -E "$2" > $D/out/$1.log 2>&1
code=$?
{ echo "# $1  filter: $2  exit: $code"
  grep -E "MODGUEST|OVERDRIVE_GUEST_KIT_TIMING|OVERDRIVE_METAL_RUN_ID=|OVERDRIVE_METAL_RUN_COMPLETE|^ +(PASS|FAIL|SIGINT|SIGKILL|LEAK|TIMEOUT|CANCEL)|Summary|error:|Cancelling|Canceling" $D/out/$1.log
} > $D/evidence/$1.txt
cat $D/evidence/$1.txt
exit $code
