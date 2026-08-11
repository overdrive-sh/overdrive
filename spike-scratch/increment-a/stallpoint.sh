#!/usr/bin/env bash
# PROBE increment-a — where exactly does the nested boot freeze?
#
# Hypothesis:        the stall is a random early-boot freeze of the nested
#                    guest, not a deterministic failure at one driver.
# Predicted outcome: the last console line differs run to run, and host dmesg
#                    shows KVM/nested complaints around the freeze.
# Falsification:     every stall freezes at the SAME line -> a real, specific
#                    driver/device bug that must be reported as such.
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
N=${N:-5}
PLACEMENT=${PLACEMENT:-host}

dmesg -C 2>/dev/null || true
for i in $(seq 1 "$N"); do
  "$HERE/run.sh" "$PLACEMENT" >/dev/null 2>&1
  rc=$?
  CON="/run/spike-increment-a/$PLACEMENT/console.log"
  if grep -q "reboot: Power down" "$CON" 2>/dev/null; then
    echo "run $i: OK (booted to power down, $(wc -l <"$CON") console lines)"
  else
    echo "run $i: STALL — last 4 console lines of $(wc -l <"$CON" 2>/dev/null) total:"
    tail -4 "$CON" 2>/dev/null | sed 's/^/      | /'
  fi
  pkill -9 -f "cloud-hypervisor .*spike-increment-a" 2>/dev/null
  sleep 1
done
echo
echo "=== host dmesg during the above runs (kvm/nested/arm) ==="
dmesg 2>/dev/null | grep -iE "kvm|nested|vcpu|WARN|BUG|trap" | tail -25 || echo "(nothing)"
