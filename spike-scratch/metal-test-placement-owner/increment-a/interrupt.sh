#!/usr/bin/env bash
# Usage: interrupt.sh <label>   V-4 with the REAL anchor: hold a run open on an
# owner anchored to the run's cargo-nextest, SIGKILL nextest, watch the box.
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-owner/increment-a
set -a; . ./.env; set +a
label=$1
OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=3600 cargo xtask metal run -- cargo nextest run --manifest-path $D/Cargo.toml --no-fail-fast \
  --success-output immediate --failure-output immediate -E 'test(=v4_real_anchor_hold)' > $D/out/$label.log 2>&1 &
LOCAL=$!
R=""
for i in $(seq 1 600); do
  R=$(grep -o 'OVERDRIVE_METAL_RUN_ID=[0-9a-f]\{24\}' $D/out/$label.log | head -1 | cut -d= -f2)
  [ -n "$R" ] && break; sleep 1
done
echo "run id: $R"
for i in $(seq 1 600); do
  ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" "sudo -n test -e /run/ovd-gk/$R/shared/v4r.ready" && break; sleep 1
done
echo "ready file seen"; sleep 2
ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s "$R" < $D/kill-nextest-watch.sh | tee $D/evidence/$label.watch.txt
wait $LOCAL; echo "local metal run exit: $?"
{ echo "# $label  run: $R"; grep -E "OWNERPROBE|OVERDRIVE_METAL_RUN_COMPLETE|Summary|error:|Cancel" $D/out/$label.log; } > $D/evidence/$label.txt
cat $D/evidence/$label.txt
echo "$R" > $D/out/$label.runid
