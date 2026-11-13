#!/usr/bin/env bash
# Usage: interrupt.sh <label> <SIGNAL> <nextest|broker>
# Starts the slow shared test (in-guest body sleeps 45 s), waits until its
# broker is up, sends SIGNAL to the named process of THIS run only (matched by
# OVERDRIVE_METAL_RUN_ID in its environment), then inspects the box.
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-module-guest/increment-a
set -a; . ./.env; set +a
label=$1 sig=$2 who=$3
cargo xtask metal run -- cargo nextest run --manifest-path $D/Cargo.toml --no-fail-fast \
  --success-output immediate --failure-output immediate -E 'test(=shared::t_slow)' > $D/out/$label.log 2>&1 &
LOCAL=$!
R=""
for i in $(seq 1 60); do
  R=$(grep -o 'OVERDRIVE_METAL_RUN_ID=[0-9a-f]\{24\}' $D/out/$label.log | head -1 | cut -d= -f2)
  [ -n "$R" ] && break; sleep 1
done
echo "run id: $R"
# wait for the broker of this run to exist
for i in $(seq 1 90); do
  n=$(ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s < $D/procs.sh | grep modguest-broker | head -1 | awk '{print $1}')
  [ -n "$n" ] && break; sleep 1
done
echo "broker pid before signal: ${n:-none}"
sleep 6
ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s "$R" "$sig" "$who" < $D/kill-and-watch.sh | tee $D/evidence/$label.watch.txt
wait $LOCAL; echo "local metal run exit: $?"
{ echo "# $label  signal: $sig -> $who  run: $R"; grep -E "MODGUEST|^ +(PASS|FAIL|SIGINT|SIGKILL|LEAK|TIMEOUT|CANCEL)|Summary|error:|Cancel|OVERDRIVE_METAL_RUN_COMPLETE" $D/out/$label.log; } > $D/evidence/$label.txt
cat $D/evidence/$label.txt
echo "$R" > $D/out/$label.runid
