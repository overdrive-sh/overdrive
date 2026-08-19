#!/usr/bin/env bash
# PROBE increment-d — population diff for the 12/12 boot stall in `full` mode.
#
# increment-a measured the nested-virt stall at roughly 1 boot in 3 succeeding,
# and increment-c's fully-confined boot succeeded on attempt 1 and then 3 times
# out of 5. `full` mode stalled 12 out of 12, always at the same place: right
# after `virtio_blk virtio0: [vda] ... blocks`, i.e. the root-mount boundary.
# 12/12 is not the same distribution as 1-in-3; treating it as "just the stall"
# would be assuming the answer.
#
# Hypothesis:        one of the things `full` adds — shared=on (memfd-backed
#                    guest RAM), the vhost-user fs devices, or the larger
#                    increment-d rootfs/init — makes the boot stall, rather than
#                    the background nested-virt flake.
# Predicted outcome: the variants separate — at least one configuration boots
#                    repeatedly while `full` does not.
# Falsification:     every variant stalls at the same rate as `full` (=> the
#                    environment degraded, not the configuration), or every
#                    variant including `full` boots (=> it was the flake and the
#                    12/12 was bad luck).
#
# Each trial is a bounded boot attempt; the only signal wanted is whether the
# guest reaches /init.
set -uo pipefail

TRIES="${1:-6}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-d
KERNEL="$A_OUT/Image"
CMDLINE="root=/dev/vda rw console=ttyAMA0 init=/init panic=1 loglevel=7"
VIRTIOFSD=/usr/libexec/virtiofsd
VMM_USER=spikevmm
VMM_GID=6001
VOLRW="$OUT/volsrc-rw"
VOLRO="$OUT/volsrc-ro"

KVM_ORIG_MODE="$(stat -c %a /dev/kvm)"
cleanup() {
  pkill -9 -f "virtiofsd --socket-path=/run/spike-d-stall" 2>/dev/null
  chmod "$KVM_ORIG_MODE" /dev/kvm 2>/dev/null || true
}
trap cleanup EXIT
chown root:kvm /dev/kvm; chmod 0660 /dev/kvm

echo "### uname -r: $(uname -r)  CH: $(cloud-hypervisor --version)  virtiofsd: $($VIRTIOFSD --version)"
echo "### rootfs-a (increment-a, 64 MiB): $(stat -c %s $A_OUT/rootfs.ext4)"
echo "### rootfs-d (increment-d, 96 MiB): $(stat -c %s $OUT/rootfs.ext4)"
echo

mkdir -p "$VOLRW" "$VOLRO"
printf 'HOST-WROTE-THIS-9876543210-zyxwvutsrq\n' >"$VOLRW/from-host.txt"
printf 'PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE\n' >"$VOLRO/preexisting-host-file.txt"
chmod -R 0777 "$VOLRW" "$VOLRO"

# rootfs_src: a = increment-a's (init has no fs work), d = increment-d's
variant() {
  local name="$1" rootfs_src="$2" mem="$3" want_fs="$4"
  local booted=0 n
  for n in $(seq 1 "$TRIES"); do
    local run="/run/spike-d-stall/$name-$n"
    pkill -9 -f "cloud-hypervisor --cpus boot=" 2>/dev/null
    pkill -9 -f "virtiofsd --socket-path=/run/spike-d-stall" 2>/dev/null
    sleep 0.3
    rm -rf "$run"; mkdir -p "$run"
    case "$rootfs_src" in
      a) cp "$A_OUT/rootfs.ext4" "$run/rootfs.ext4" ;;
      d) cp "$OUT/rootfs.ext4"   "$run/rootfs.ext4" ;;
    esac
    chown -R 6001:6001 "$run"; chmod 0700 "$run"

    local argv=(cloud-hypervisor --cpus boot=1 --memory "$mem"
      --kernel "$KERNEL" --cmdline "$CMDLINE"
      --disk "path=$run/rootfs.ext4"
      --serial "file=$run/console.log" --console off
      --api-socket "path=$run/ch-api.sock"
      --seccomp true --landlock)
    local rules=("path=$run,access=rw")
    if [ "$want_fs" = 1 ]; then
      $VIRTIOFSD --socket-path="$run/volrw.sock" --shared-dir="$VOLRW" --tag=volrw \
        --cache=never --sandbox=namespace --socket-group="$VMM_USER" \
        --log-level=info >"$run/fsd.log" 2>&1 &
      for _ in $(seq 1 60); do [ -S "$run/volrw.sock" ] && break; sleep 0.1; done
      argv+=(--fs "tag=volrw,socket=$run/volrw.sock")
    fi
    argv+=(--landlock-rules "${rules[@]}")

    timeout 70 \
      prlimit --fsize=$((1024*1024*1024)) --nofile=256 -- \
      setpriv --reuid="$VMM_USER" --regid="$VMM_GID" --init-groups --no-new-privs -- \
      "${argv[@]}" >"$run/ch.log" 2>&1
    if grep -q 'HELLO from overdrive' "$run/console.log" 2>/dev/null; then
      booted=$((booted+1))
    fi
    pkill -9 -f "virtiofsd --socket-path=$run" 2>/dev/null
  done
  local last="/run/spike-d-stall/$name-$TRIES/console.log"
  printf '  %-28s rootfs=%s mem=%-22s fs=%s  ->  reached /init %d/%d   last stall point: %s\n' \
    "$name" "$rootfs_src" "$mem" "$want_fs" "$booted" "$TRIES" \
    "$(tail -1 "$last" 2>/dev/null | head -c 70)"
}

echo "=== is it the rootfs/init, the memory backing, or the fs device?"
variant a-plain        a "size=512M"           0
variant a-sharedon     a "size=512M,shared=on" 0
variant d-plain        d "size=512M"           0
variant d-sharedon     d "size=512M,shared=on" 0
variant d-sharedon-fs  d "size=512M,shared=on" 1
echo
echo "=== console tail of one d-sharedon-fs attempt (the full-mode shape)"
tail -3 "/run/spike-d-stall/d-sharedon-fs-1/console.log" 2>/dev/null | sed 's/^/    /'
echo "=== virtiofsd log from that attempt"
sed 's/^/    /' "/run/spike-d-stall/d-sharedon-fs-1/fsd.log" 2>/dev/null
exit 0
