#!/usr/bin/env bash
# Materialize the public-ingress-gateway example's VM Service on qualified
# native metal: a private kernel copy, a private reflinked rootfs carrying the
# static guest HTTP server, and nothing else. It follows the marker-owned
# preparation convention of examples/service-kind-vm-workloads/prepare.sh.
# Serve state (config, data, credentials) is owned by run-example.sh under its
# argv scratch directory, not here.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly STAGING_ROOT="/srv/vm/overdrive-testing"
readonly OUTPUT_ROOT="$STAGING_ROOT/pig-example"
readonly OWNERSHIP_MARKER="$OUTPUT_ROOT/.pig-example-owned"
readonly MARKER_PREFIX="pig-example-owned-v1"
readonly BASE_KERNEL="${PIG_EXAMPLE_BASE_KERNEL:-$STAGING_ROOT/kernel}"
readonly BASE_ROOTFS="${PIG_EXAMPLE_BASE_ROOTFS:-$STAGING_ROOT/rootfs.ext4}"
readonly KERNEL="$OUTPUT_ROOT/kernel"
readonly ROOTFS="$OUTPUT_ROOT/rootfs.ext4"
readonly SERVER="$OUTPUT_ROOT/api-server"
readonly MOUNT_DIR="$OUTPUT_ROOT/mnt"
readonly GUEST_SERVER="/opt/overdrive/examples/pig/api-server"
readonly STATIC_TARGET="x86_64-unknown-linux-musl"
readonly OWNERSHIP_TOKEN="${PIG_EXAMPLE_OWNERSHIP_TOKEN:-}"

LOOP_DEVICE=""
PREPARE_COMMITTED=0
PREPARE_OWNS_OUTPUT=0

die() {
  echo "pig-example prepare: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
usage: prepare.sh check-source|prepare|check|cleanup

  check-source  validate checked-in sources and fixed spec paths (host-safe)
  prepare       materialize the fixed native-metal runtime paths
  check         verify the materialization, including the guest binary
  cleanup       remove only the marker-owned fixed materialization

Optional base-image overrides:
  PIG_EXAMPLE_BASE_KERNEL=/absolute/path/to/kernel
  PIG_EXAMPLE_BASE_ROOTFS=/absolute/path/to/rootfs.ext4
USAGE
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

validate_paths() {
  [[ "$BASE_KERNEL" = /* && "$BASE_ROOTFS" = /* ]] \
    || die "base kernel and rootfs paths must be absolute"
  [[ "$BASE_ROOTFS" != "$ROOTFS" ]] \
    || die "base rootfs and private rootfs must be different files"
}

check_source() {
  validate_paths
  local item
  for item in README.md guest_server.rs prepare.sh prepare-test-ca.sh \
    run-example.sh service.toml route.toml.template; do
    [[ -f "$EXAMPLE_DIR/$item" ]] \
      || die "missing checked-in source: $EXAMPLE_DIR/$item"
  done
  local spec="$EXAMPLE_DIR/service.toml"
  [[ "$(grep -Fxc '[service]' "$spec")" -eq 1 ]] \
    || die "Service fixture must declare exactly one [service]"
  [[ "$(grep -Fxc '[vm]' "$spec")" -eq 1 ]] \
    || die "Service fixture must declare exactly one [vm]"
  if grep -Eq '^\[exec\]' "$spec"; then
    die "Service fixture must not declare [exec]"
  fi
  grep -Fxq 'id = "api"' "$spec" || die "Service id must be api"
  grep -Fq "command = \"$GUEST_SERVER\"" "$spec" \
    || die "guest command differs from preparation contract"
  grep -Fq "kernel = \"$KERNEL\"" "$spec" \
    || die "kernel path differs from preparation contract"
  grep -Fq "rootfs = \"$ROOTFS\"" "$spec" \
    || die "rootfs path differs from preparation contract"
  [[ "$(grep -Fo '__PUBLIC_HOSTNAME__' "$EXAMPLE_DIR/route.toml.template" | wc -l | tr -d ' ')" -eq 1 ]] \
    || die "Route template must contain exactly one __PUBLIC_HOSTNAME__"
  grep -Fxq 'service = "api"' "$EXAMPLE_DIR/route.toml.template" \
    || die "Route template target.service must be api"
  grep -Fq 'X-Workload: api' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server X-Workload marker is absent"
  grep -Fq 'PIG-STREAM-CHUNK-' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server stream body is absent"
  grep -Fq 'requests=' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server request-count marker is absent"
}

require_native_linux() {
  [[ "$(uname -s)" == "Linux" ]] || die "materialization is Linux-only"
  [[ "$(uname -m)" == "x86_64" ]] || die "materialization requires x86_64"
  [[ "$(id -u)" -eq 0 ]] || die "prepare/check/cleanup must run as root on metal"
}

verify_static_binary() {
  local binary="$1"
  [[ -x "$binary" ]] || die "binary is not executable: $binary"
  file "$binary" | grep -Eq 'statically linked|static-pie linked' \
    || die "binary is not statically linked: $binary"
  if readelf -l "$binary" | grep -q 'INTERP'; then
    die "binary unexpectedly carries a dynamic interpreter: $binary"
  fi
}

unmount_private_rootfs() {
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

verify_marker() {
  [[ -f "$OWNERSHIP_MARKER" ]] || die "missing ownership marker: $OWNERSHIP_MARKER"
  local marker
  marker="$(<"$OWNERSHIP_MARKER")"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$marker" == "$MARKER_PREFIX:$OWNERSHIP_TOKEN" ]] \
      || die "materialization belongs to another invocation"
  else
    case "$marker" in
      "$MARKER_PREFIX"|"$MARKER_PREFIX":*) ;;
      *) die "unexpected ownership marker content" ;;
    esac
  fi
}

remove_owned_output() {
  [[ -e "$OUTPUT_ROOT" ]] || return 0
  [[ -f "$OWNERSHIP_MARKER" ]] \
    || die "refusing to remove unmarked path: $OUTPUT_ROOT"
  verify_marker
  if mountpoint -q "$MOUNT_DIR"; then
    die "refusing to remove $OUTPUT_ROOT while $MOUNT_DIR is mounted"
  fi
  rm -rf -- "$OUTPUT_ROOT"
}

remove_process_owned_partial_output() {
  [[ "$PREPARE_OWNS_OUTPUT" -eq 1 ]] || return 0
  [[ -e "$OUTPUT_ROOT" ]] || return 0
  if mountpoint -q "$MOUNT_DIR"; then
    die "refusing to remove process-owned partial output while $MOUNT_DIR is mounted"
  fi
  rm -rf -- "$OUTPUT_ROOT"
}

on_prepare_exit() {
  local rc=$?
  trap - EXIT HUP INT TERM
  if ! unmount_private_rootfs; then
    echo "pig-example prepare: bounded mount/loop cleanup failed; leaving process-owned output for inspection" >&2
    exit 1
  fi
  if [[ "$rc" -ne 0 && "$PREPARE_COMMITTED" -ne 1 ]]; then
    remove_process_owned_partial_output
  fi
  exit "$rc"
}

mount_private_rootfs() {
  LOOP_DEVICE="$(losetup --find --show "$ROOTFS")"
  timeout 15s mount "$LOOP_DEVICE" "$MOUNT_DIR"
}

verify_materialization_without_mount() {
  verify_marker
  [[ -r "$KERNEL" && -f "$ROOTFS" ]] || die "private kernel/rootfs is incomplete"
  verify_static_binary "$SERVER"
  [[ "$(stat -c %a "$OUTPUT_ROOT")" == "711" ]] \
    || die "output root must be mode 0711 for confined-VMM traversal"
  setpriv --reuid=4200 --regid=4200 --clear-groups test -x "$OUTPUT_ROOT" \
    || die "uid 4200 cannot traverse the output root and its ancestors"
}

prepare() {
  require_native_linux
  check_source
  local command
  for command in chmod cp dirname file grep install losetup mount mountpoint \
    readelf rustc setpriv stat sync timeout umount; do
    require_command "$command"
  done
  [[ -r "$BASE_KERNEL" && -f "$BASE_KERNEL" ]] \
    || die "base kernel is absent or unreadable: $BASE_KERNEL"
  [[ -r "$BASE_ROOTFS" && -f "$BASE_ROOTFS" ]] \
    || die "base rootfs is absent or unreadable: $BASE_ROOTFS"
  [[ ! -e "$OUTPUT_ROOT" ]] \
    || die "$OUTPUT_ROOT already exists; run prepare.sh cleanup first"
  rustc --print target-libdir --target "$STATIC_TARGET" >/dev/null 2>&1 \
    || die "Rust target $STATIC_TARGET is unavailable (install it before preparing)"

  trap on_prepare_exit EXIT
  trap 'exit 130' HUP INT TERM
  PREPARE_OWNS_OUTPUT=1
  install -d -m 0711 "$OUTPUT_ROOT"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$OWNERSHIP_TOKEN" =~ ^[A-Za-z0-9._-]+$ ]] \
      || die "ownership token contains unsupported characters"
    printf '%s:%s\n' "$MARKER_PREFIX" "$OWNERSHIP_TOKEN" >"$OWNERSHIP_MARKER"
  else
    printf '%s\n' "$MARKER_PREFIX" >"$OWNERSHIP_MARKER"
  fi
  install -d -m 0755 "$MOUNT_DIR"

  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/guest_server.rs" -o "$SERVER"
  chmod 0755 "$SERVER"
  verify_static_binary "$SERVER"

  cp --reflink=auto --sparse=auto "$BASE_KERNEL" "$KERNEL"
  chmod 0644 "$KERNEL"
  cp --reflink=always --sparse=auto "$BASE_ROOTFS" "$ROOTFS"
  chmod 0600 "$ROOTFS"

  mount_private_rootfs
  install -d -m 0755 "$MOUNT_DIR$(dirname "$GUEST_SERVER")"
  install -m 0755 "$SERVER" "$MOUNT_DIR$GUEST_SERVER"
  verify_static_binary "$MOUNT_DIR$GUEST_SERVER"
  sync "$MOUNT_DIR$GUEST_SERVER"
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"

  verify_materialization_without_mount
  PREPARE_COMMITTED=1
  PREPARE_OWNS_OUTPUT=0
  trap - EXIT HUP INT TERM
  echo "prepared public-ingress example materialization at $OUTPUT_ROOT"
}

check() {
  require_native_linux
  check_source
  local command
  for command in file losetup mount mountpoint readelf setpriv stat timeout umount; do
    require_command "$command"
  done
  verify_materialization_without_mount
  PREPARE_COMMITTED=1
  trap on_prepare_exit EXIT
  trap 'exit 130' HUP INT TERM
  mount_private_rootfs
  verify_static_binary "$MOUNT_DIR$GUEST_SERVER"
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"
  trap - EXIT HUP INT TERM
  echo "verified public-ingress example materialization at $OUTPUT_ROOT"
}

cleanup() {
  require_native_linux
  local command
  for command in cut losetup mountpoint timeout umount; do
    require_command "$command"
  done
  if [[ -e "$OUTPUT_ROOT" ]]; then
    if mountpoint -q "$MOUNT_DIR"; then
      timeout 15s umount "$MOUNT_DIR" \
        || die "bounded unmount failed; refusing to remove mounted output"
    fi
    while IFS= read -r loop; do
      [[ -n "$loop" ]] && timeout 15s losetup -d "$loop"
    done < <(losetup -j "$ROOTFS" 2>/dev/null | cut -d: -f1)
    remove_owned_output
  fi
  echo "removed public-ingress example-owned materialization from $OUTPUT_ROOT"
}

case "${1:-}" in
  check-source) check_source ;;
  prepare) prepare ;;
  check) check ;;
  cleanup) cleanup ;;
  *) usage ;;
esac
