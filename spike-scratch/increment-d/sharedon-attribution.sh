#!/usr/bin/env bash
# PROBE increment-d — attribute the `--memory shared=on` boot failure.
#
# stalldiff.sh isolated the cause: WITHOUT shared=on the guest reaches /init
# 6/6; WITH shared=on it reaches /init 0/6, always freezing right after the
# virtio_blk probe (the root-mount boundary). The fs device and the rootfs are
# innocent.
#
# "shared=on breaks the boot" is still only the LAYER, not the mechanism
# (.claude/rules/debugging.md § 2). Before this is reported as a verdict against
# [D8b], three attributions must be separated, because they have very different
# consequences for the feature:
#
#   (i)  a confinement interaction   -> [D7] x [D8] compose badly; fixable
#   (ii) a CH option interaction     -> a different --memory shape works
#   (iii) the nested aarch64 host    -> a DEV-HOST artifact; says nothing about
#                                        x86_64 production, and P6 is UNPROVEN
#                                        here rather than refuted
#
# Hypothesis:        (iii) — MAP_SHARED guest memory does not survive nested
#                    KVM on Apple Silicon, so any VMM using it stalls.
# Predicted outcome: root+unconfined shared=on ALSO stalls (rules out i), the
#                    --memory variants also stall (rules out ii), and QEMU with
#                    a memory-backend-memfd share=on ALSO stalls while the same
#                    QEMU with private memory boots (confirms iii).
# Falsification:     root+unconfined boots (=> i), or a --memory variant boots
#                    (=> ii), or QEMU-with-shared-memfd boots fine (=> CH bug,
#                    which is a real P6 DOESN'T-WORK against CH v46).
set -uo pipefail

TRIES="${1:-4}"
A_OUT=/var/tmp/spike-increment-a
KERNEL="$A_OUT/Image"
ROOTFS="$A_OUT/rootfs.ext4"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VMM_USER=spikevmm
VMM_GID=6001

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
trap 'chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true' EXIT

echo "### uname -r: $(uname -r)  uname -m: $(uname -m)"
echo "### CH: $(cloud-hypervisor --version)"
echo "### qemu: $(qemu-system-aarch64 --version 2>/dev/null | head -1 || echo '<absent>')"
echo "### systemd-detect-virt: $(systemd-detect-virt 2>/dev/null)"
echo

ch_trial() {
  local name="$1" mem="$2" confined="$3"
  local booted=0 n
  for n in $(seq 1 "$TRIES"); do
    local run="/run/spike-d-attr/$name-$n"
    pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null; sleep 0.2
    rm -rf "$run"; mkdir -p "$run"; cp "$ROOTFS" "$run/rootfs.ext4"
    local argv=(cloud-hypervisor --cpus boot=1 --memory "$mem"
      --kernel "$KERNEL" --cmdline "$CMDLINE" --disk "path=$run/rootfs.ext4"
      --serial "file=$run/console.log" --console off)
    local prefix=()
    if [ "$confined" = 1 ]; then
      chmod 0660 /dev/kvm; chown root:kvm /dev/kvm
      chown -R 6001:6001 "$run"; chmod 0700 "$run"
      argv+=(--seccomp true --landlock --landlock-rules "path=$run,access=rw")
      prefix=(prlimit --fsize=$((1024*1024*1024)) --nofile=256 --
              setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs --)
    else
      chmod 0666 /dev/kvm
    fi
    timeout 60 "${prefix[@]}" "${argv[@]}" >"$run/ch.log" 2>&1
    grep -q 'HELLO from overdrive' "$run/console.log" 2>/dev/null && booted=$((booted+1))
  done
  printf '  %-34s confined=%s  memory=%-34s -> /init %d/%d   %s\n' \
    "$name" "$confined" "$mem" "$booted" "$TRIES" \
    "$(tail -1 "/run/spike-d-attr/$name-$TRIES/console.log" 2>/dev/null | head -c 62)"
}

echo "=== (i) is it a CONFINEMENT interaction? root + unconfined vs confined"
ch_trial root-plain          "size=512M"                      0
ch_trial root-sharedon       "size=512M,shared=on"            0
ch_trial confined-sharedon   "size=512M,shared=on"            1
echo
echo "=== (ii) is some other --memory shape workable? (all root+unconfined)"
ch_trial sharedon-prefault   "size=512M,shared=on,prefault=on" 0
ch_trial sharedon-thpoff     "size=512M,shared=on,thp=off"     0
ch_trial sharedon-128M       "size=128M,shared=on"             0
ch_trial plain-prefault      "size=512M,prefault=on"           0
echo
echo "=== (iii) QEMU cross-check: is MAP_SHARED guest RAM viable AT ALL here?"
qemu_trial() {
  local name="$1"; shift
  local booted=0 n
  for n in $(seq 1 "$TRIES"); do
    local run="/run/spike-d-attr/$name-$n"
    pkill -9 -f qemu-system-aarch64 2>/dev/null; sleep 0.2
    rm -rf "$run"; mkdir -p "$run"; cp "$ROOTFS" "$run/rootfs.ext4"
    local args=()
    for a in "$@"; do args+=("${a//__RUN__/$run}"); done
    timeout 60 qemu-system-aarch64 \
      -machine virt,accel=kvm -cpu host -smp 1 \
      -kernel "$KERNEL" -append "$CMDLINE" \
      -drive "file=$run/rootfs.ext4,format=raw,if=virtio" \
      -nographic -serial "file:$run/console.log" -monitor none \
      "${args[@]}" >"$run/qemu.log" 2>&1
    grep -q 'HELLO from overdrive' "$run/console.log" 2>/dev/null && booted=$((booted+1))
  done
  printf '  %-34s -> /init %d/%d   %s\n' "$name" "$booted" "$TRIES" \
    "$(tail -1 "/run/spike-d-attr/$name-$TRIES/console.log" 2>/dev/null | head -c 62)"
}
chmod 0666 /dev/kvm
if command -v qemu-system-aarch64 >/dev/null; then
  qemu_trial qemu-private-mem  -m 512
  qemu_trial qemu-memfd-shared \
    -object "memory-backend-memfd,id=mem,size=512M,share=on" \
    -machine "virt,accel=kvm,memory-backend=mem"
  echo "--- qemu-memfd-shared stderr (attempt 1):"
  sed 's/^/    /' /run/spike-d-attr/qemu-memfd-shared-1/qemu.log 2>/dev/null | head -5
else
  echo "  qemu-system-aarch64 absent — cross-check not available"
fi
exit 0
