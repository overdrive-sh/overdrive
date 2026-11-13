#!/usr/bin/env bash
# SIGKILL the `shared` module's broker while a second module's test keeps the
# run alive, then watch whether the killed broker's cloud-hypervisor survives
# inside the still-running run.
set -u
cd "$(dirname "$0")/../../.."
D=spike-scratch/metal-test-placement-module-guest/increment-a
set -a; . ./.env; set +a
label=interrupt-sigkill-broker-midrun
cargo xtask metal run -- cargo nextest run --manifest-path $D/Cargo.toml --no-fail-fast \
  --success-output immediate --failure-output immediate -E 'test(=shared::t_slow) | test(=shared_b::u_slow)' > $D/out/$label.log 2>&1 &
LOCAL=$!
R=""
for i in $(seq 1 60); do
  R=$(grep -o 'OVERDRIVE_METAL_RUN_ID=[0-9a-f]\{24\}' $D/out/$label.log | head -1 | cut -d= -f2); [ -n "$R" ] && break; sleep 1
done
echo "run id: $R"
for i in $(seq 1 120); do
  n=$(ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s < $D/procs.sh | grep -c modguest-broker)
  [ "${n:-0}" = 2 ] && break; sleep 1
done
sleep 8   # both brokers are up and both slow bodies are running
snap() { echo "--- $1"; ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s < $D/procs.sh; }
snap "before kill (expect 2 brokers + >=2 cloud-hypervisor)"
B=$(ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" bash -s < $D/procs.sh | grep 'modguest-broker' | head -1 | awk '{print $1}')
echo "killing -9 the first of the two brokers (which module it served is in the client lines below): pid $B"
ssh -o BatchMode=yes "$OVERDRIVE_METAL_TARGET" "sudo -n kill -9 $B"
sleep 3; snap "3s after kill"
sleep 5; snap "8s after kill"
wait $LOCAL; echo "local metal run exit: $?"
{ echo "# $label run: $R"; grep -E "MODGUEST_HOST|^ +(PASS|FAIL)|Summary|OVERDRIVE_METAL_RUN_COMPLETE" $D/out/$label.log; } > $D/evidence/$label.txt
echo "$R" > $D/out/$label.runid; cat $D/evidence/$label.txt
