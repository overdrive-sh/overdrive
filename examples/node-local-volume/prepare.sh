#!/usr/bin/env bash
# Materialize the node-local volume example on qualified native metal.
#
# This does not duplicate the service-kind VM bundle's preparation. It runs
# that bundle's prepare.sh (kernel, private rootfs, overdrive-init and module
# closure, serve data directory, KEK credential, ownership marker, bounded
# cleanup) and then adds the one guest program this example owns,
# guest_volume.rs, to the private rootfs. The specs here therefore name the
# same kernel and rootfs paths, and the two examples cannot be prepared at the
# same time.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly BASE_PREPARE="$EXAMPLE_DIR/../service-kind-vm-workloads/prepare.sh"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
readonly ROOTFS="$OUTPUT_ROOT/rootfs.ext4"
readonly MOUNT_DIR="$OUTPUT_ROOT/mnt"
readonly GUEST_NOTES="/opt/overdrive/examples/svm/volume-notes"
readonly HOST_NOTES="$OUTPUT_ROOT/volume-notes"
readonly STATIC_TARGET="x86_64-unknown-linux-musl"

LOOP_DEVICE=""
INSTALLING=0

die() {
  echo "node-local-volume prepare: $*" >&2
  exit 1
}

check_source() {
  local item spec
  for item in README.md guest_volume.rs prepare.sh run-example.sh \
    volume-service.toml volume-oversized.toml; do
    [[ -f "$EXAMPLE_DIR/$item" ]] || die "missing checked-in source: $EXAMPLE_DIR/$item"
  done
  for spec in volume-service.toml volume-oversized.toml; do
    spec="$EXAMPLE_DIR/$spec"
    [[ "$(grep -Fxc '[service]' "$spec")" -eq 1 ]] || die "must declare exactly one [service]: $spec"
    [[ "$(grep -Fxc '[vm]' "$spec")" -eq 1 ]] || die "must declare exactly one [vm]: $spec"
    [[ "$(grep -Fxc '[vm.volume]' "$spec")" -eq 1 ]] || die "must declare exactly one [vm.volume]: $spec"
    grep -Fxq 'replicas = 1' "$spec" || die "a volume requires replicas = 1: $spec"
    grep -Fq "command = \"$GUEST_NOTES\"" "$spec" || die "guest command differs from the preparation contract: $spec"
    grep -Fq "kernel = \"$OUTPUT_ROOT/kernel\"" "$spec" || die "kernel differs from the preparation contract: $spec"
    grep -Fq "rootfs = \"$ROOTFS\"" "$spec" || die "rootfs differs from the preparation contract: $spec"
  done
  grep -Fq 'size = "1GiB"' "$EXAMPLE_DIR/volume-service.toml" || die "the main spec must declare a 1GiB volume"
  grep -Fq 'size = "1000TiB"' "$EXAMPLE_DIR/volume-oversized.toml" || die "the oversized spec must declare a 1000TiB volume"
  grep -Fq 'ovd-volume' "$EXAMPLE_DIR/guest_volume.rs" || die "guest does not look for the volume serial"
  "$BASE_PREPARE" check-source
}

unmount_rootfs() {
  local failed=0
  if mountpoint -q "$MOUNT_DIR"; then
    timeout 15s umount "$MOUNT_DIR" || failed=1
  fi
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
  if [[ "$rc" -ne 0 && "$INSTALLING" -eq 1 ]]; then
    "$BASE_PREPARE" cleanup || true
  fi
  exit "$rc"
}

prepare() {
  check_source
  for command in file install losetup mount mountpoint readelf rustc sync timeout umount; do
    command -v "$command" >/dev/null 2>&1 || die "required command is unavailable: $command"
  done
  "$BASE_PREPARE" prepare
  INSTALLING=1
  trap on_install_exit EXIT
  trap 'exit 130' HUP INT TERM
  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/guest_volume.rs" -o "$HOST_NOTES"
  chmod 0755 "$HOST_NOTES"
  file "$HOST_NOTES" | grep -Eq 'statically linked|static-pie linked' \
    || die "guest program is not statically linked: $HOST_NOTES"
  LOOP_DEVICE="$(losetup --find --show "$ROOTFS")"
  timeout 15s mount "$LOOP_DEVICE" "$MOUNT_DIR"
  install -m 0755 "$HOST_NOTES" "$MOUNT_DIR$GUEST_NOTES"
  sync "$MOUNT_DIR$GUEST_NOTES"
  unmount_rootfs || die "bounded rootfs unmount/loop detach failed"
  INSTALLING=0
  trap - EXIT HUP INT TERM
  echo "prepared node-local volume example materialization at $OUTPUT_ROOT"
}

case "${1:-}" in
  check-source) check_source ;;
  prepare) prepare ;;
  cleanup) "$BASE_PREPARE" cleanup ;;
  *) die 'usage: prepare.sh check-source|prepare|cleanup' ;;
esac
