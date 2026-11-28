#!/usr/bin/env bash
# Materialize the E17 capture on qualified native metal: the service-kind VM
# bundle's private kernel/rootfs/serve state (its prepare.sh), plus the one guest
# program this capture runs, the guest kit's static overdrive-guest-peer, copied
# into the private rootfs. Nothing is installed in the guest's network
# configuration: the guest keeps the resolv.conf its own init writes.
set -euo pipefail

CAPTURE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly CAPTURE_DIR
REPO_ROOT="$(cd "$CAPTURE_DIR/../../.." && pwd)"
readonly REPO_ROOT
readonly BASE_PREPARE="$REPO_ROOT/examples/service-kind-vm-workloads/prepare.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
readonly ROOTFS="$OUTPUT_ROOT/rootfs.ext4"
readonly MOUNT_DIR="$OUTPUT_ROOT/mnt"
readonly GUEST_PEER="/opt/overdrive/examples/svm/overdrive-guest-peer"
readonly HOST_PEER="$REPO_ROOT/target/x86_64-unknown-linux-musl/release/overdrive-guest-peer"

LOOP_DEVICE=""
INSTALLING=0

die() { echo "E17 prepare: $*" >&2; exit 1; }

unmount_rootfs() {
  local failed=0
  if mountpoint -q "$MOUNT_DIR"; then timeout 15s umount "$MOUNT_DIR" || failed=1; fi
  if [[ -n "$LOOP_DEVICE" ]] && losetup "$LOOP_DEVICE" >/dev/null 2>&1; then
    timeout 15s losetup -d "$LOOP_DEVICE" || failed=1
  fi
  LOOP_DEVICE=""
  return "$failed"
}

on_install_exit() {
  local rc=$?
  trap - EXIT HUP INT TERM
  unmount_rootfs || rc=1
  if [[ "$rc" -ne 0 && "$INSTALLING" -eq 1 ]]; then "$BASE_PREPARE" cleanup || true; fi
  exit "$rc"
}

prepare() {
  [[ -x "$HOST_PEER" ]] || die "guest kit not prepared: $HOST_PEER (cargo xtask guest-kit-prepare)"
  file "$HOST_PEER" | grep -Eq 'statically linked|static-pie linked' || die "not static: $HOST_PEER"
  "$BASE_PREPARE" check-source
  "$BASE_PREPARE" prepare
  INSTALLING=1
  trap on_install_exit EXIT
  trap 'exit 130' HUP INT TERM
  LOOP_DEVICE="$(losetup --find --show "$ROOTFS")"
  timeout 15s mount "$LOOP_DEVICE" "$MOUNT_DIR"
  install -m 0755 "$HOST_PEER" "$MOUNT_DIR$GUEST_PEER"
  sync "$MOUNT_DIR$GUEST_PEER"
  # Which cat the guest image carries (recorded by the runner; empty if none).
  : >"$OUTPUT_ROOT/guest-cat-path"
  local candidate
  for candidate in /bin/cat /usr/bin/cat; do
    if [[ -x "$MOUNT_DIR$candidate" ]]; then echo "$candidate" >"$OUTPUT_ROOT/guest-cat-path"; break; fi
  done
  unmount_rootfs || die "bounded rootfs unmount/loop detach failed"
  INSTALLING=0
  trap - EXIT HUP INT TERM
  echo "prepared E17 materialization at $OUTPUT_ROOT"
}

case "${1:-}" in
  prepare) prepare ;;
  cleanup) "$BASE_PREPARE" cleanup ;;
  *) die 'usage: prepare.sh prepare|cleanup' ;;
esac
