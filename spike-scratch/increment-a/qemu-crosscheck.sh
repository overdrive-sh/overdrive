#!/usr/bin/env bash
# PROBE increment-a — cross-check the intermittent stall against a DIFFERENT VMM.
#
# Hypothesis:        the intermittent early-boot freeze is a property of the
#                    nested Apple-VZ -> Lima -> KVM(nVHE) environment, not of
#                    cloud-hypervisor.
# Predicted outcome: QEMU/KVM booting the SAME Image + SAME ext4 rootfs stalls
#                    at a comparable rate.
# Falsification:     QEMU is 100% clean over the same N -> the stall is
#                    cloud-hypervisor's and P1 must be reported with that caveat.
set -uo pipefail
OUT=/var/tmp/spike-increment-a
RUN=/run/spike-increment-a/qemu
N=${N:-8}
rm -rf "$RUN"; mkdir -p "$RUN"

qemu-system-aarch64 --version | head -1
pass=0; fail=0
for i in $(seq 1 "$N"); do
  cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"
  : > "$RUN/console.log"
  timeout -k 5 90 qemu-system-aarch64 \
    -machine virt,accel=kvm -cpu host -smp 1 -m 512 \
    -kernel "$OUT/Image" \
    -append "root=/dev/vda rw console=ttyAMA0 init=/init panic=1" \
    -drive "file=$RUN/rootfs.ext4,format=raw,if=virtio" \
    -display none -serial "file:$RUN/console.log" -no-reboot \
    >"$RUN/qemu-stderr.log" 2>&1
  rc=$?
  if grep -q "reboot: Power down" "$RUN/console.log" 2>/dev/null; then
    pass=$((pass+1))
    printf '  qemu run %d: OK    rc=%-4s (%s console lines)\n' "$i" "$rc" "$(wc -l <"$RUN/console.log")"
  else
    fail=$((fail+1))
    printf '  qemu run %d: STALL rc=%-4s last line: %s\n' "$i" "$rc" \
      "$(tail -1 "$RUN/console.log" 2>/dev/null)"
  fi
done
echo "  => qemu/KVM, same Image + same rootfs: booted-to-poweroff $pass/$N, stalled $fail/$N"
