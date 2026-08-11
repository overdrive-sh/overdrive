#!/usr/bin/env bash
# PROBE increment-a — population diff for the intermittent guest stall.
#
# Hypothesis:        the stall is a property of the NESTED boot (Apple VZ ->
#                    Lima -> KVM nVHE -> cloud-hypervisor), not of vsock.
# Predicted outcome: the `host-novsock` population stalls at a similar rate to
#                    the `host` population.
# Falsification:     `host-novsock` is 100% clean while `host` stalls -> the
#                    vsock device is implicated and P2 is not safe to PROMOTE.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
N=${N:-6}

for placement in "$@"; do
  pass=0; fail=0
  echo "################ population: $placement  (n=$N) ################"
  for i in $(seq 1 "$N"); do
    out=$("$HERE/run.sh" "$placement" 2>&1)
    rc=$?
    last=$(grep -E "^init: |^\[HOST" <<<"$out" | tail -1)
    reached_init=$(grep -c "init: HELLO" <<<"$out")
    powered=$(grep -c "reboot: Power down" <<<"$out")
    chrc=$(grep -oP "cloud-hypervisor exit code : \K.*" <<<"$out")
    if [ "$powered" -gt 0 ]; then pass=$((pass+1)); tag=OK; else fail=$((fail+1)); tag=STALL; fi
    printf '  run %d: %-5s ch_rc=%-4s init_reached=%s powered_off=%s | last: %s\n' \
      "$i" "$tag" "$chrc" "$reached_init" "$powered" "${last:0:90}"
    # Never let a stalled VM leak into the next run.
    pkill -9 -f "cloud-hypervisor .*spike-increment-a" 2>/dev/null
    sleep 1
  done
  echo "  => $placement: booted-to-poweroff $pass/$N, stalled $fail/$N"
  echo
done
