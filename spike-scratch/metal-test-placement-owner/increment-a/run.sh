#!/usr/bin/env bash
# Usage: run.sh <label> '<nextest filterset>' [extra nextest args]
# Ordinary Metal grammar. Raw log -> out/ (ignored), filtered evidence -> evidence/<label>.txt (committed).
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-owner/increment-a
mkdir -p $D/out $D/evidence
label=$1 filter=$2; shift 2
OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=3600 cargo xtask metal run -- cargo nextest run --manifest-path $D/Cargo.toml --no-fail-fast "$@" \
  --success-output immediate --failure-output immediate -E "$filter" > $D/out/$label.log 2>&1
code=$?
{ echo "# $label  filter: $filter  exit: $code"
  grep -E "OWNERPROBE|^OWNER t=|OVERDRIVE_METAL_RUN_ID=|OVERDRIVE_METAL_RUN_COMPLETE|^ +(PASS|FAIL|SIGINT|SIGKILL|LEAK|TIMEOUT|CANCEL)|Summary|error(\[|:)|panicked|assertion|Cancelling|Canceling" $D/out/$label.log
} > $D/evidence/$label.txt
cat $D/evidence/$label.txt
exit $code
