#!/usr/bin/env bash
# Materialize the checked-in service-kind VM example on qualified native metal.
# This is the bundle's sole owner of runtime binaries, private appliance image,
# isolated serve state, credentials, and their cleanup.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly STAGING_ROOT="/srv/vm/overdrive-testing"
readonly OUTPUT_ROOT="$STAGING_ROOT/svm-e08"
readonly OWNERSHIP_MARKER="$OUTPUT_ROOT/.svm-e08-owned"
readonly BASE_KERNEL="${SVM_E08_BASE_KERNEL:-$STAGING_ROOT/kernel}"
readonly BASE_ROOTFS="${SVM_E08_BASE_ROOTFS:-$STAGING_ROOT/rootfs.ext4}"
readonly KERNEL="$OUTPUT_ROOT/kernel"
readonly ROOTFS="$OUTPUT_ROOT/rootfs.ext4"
readonly SERVER="$OUTPUT_ROOT/e08-server"
readonly CLIENT="$OUTPUT_ROOT/e08-client"
readonly MOUNT_DIR="$OUTPUT_ROOT/mnt"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly KEK_FILE="$CREDS_DIR/overdrive-ca-root"
readonly GUEST_SERVER="/opt/overdrive/examples/svm/e08-server"
readonly GUEST_CLIENT="/opt/overdrive/examples/svm/e08-client"
readonly STATIC_TARGET="x86_64-unknown-linux-musl"
readonly OWNERSHIP_TOKEN="${SVM_E08_OWNERSHIP_TOKEN:-}"

LOOP_DEVICE=""
PREPARE_COMMITTED=0
PREPARE_OWNS_OUTPUT=0

die() {
  echo "svm-e08 prepare: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
usage: prepare.sh check-source|prepare|check|cleanup

  check-source  validate checked-in sources and fixed spec paths (host-safe)
  prepare       materialize the fixed native-metal runtime paths
  check         verify the materialization, including both guest binaries
  cleanup       remove only the marker-owned fixed materialization

Optional base-image overrides:
  SVM_E08_BASE_KERNEL=/absolute/path/to/kernel
  SVM_E08_BASE_ROOTFS=/absolute/path/to/rootfs.ext4
USAGE
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

validate_paths() {
  [[ "$STAGING_ROOT" = /* ]] || die "staging root must be absolute"
  [[ "$STAGING_ROOT" != "/" ]] || die "refusing to use / as the staging root"
  [[ "$OUTPUT_ROOT" == "$STAGING_ROOT/svm-e08" ]] \
    || die "internal output-root invariant failed"
  [[ "$BASE_KERNEL" = /* && "$BASE_ROOTFS" = /* ]] \
    || die "base kernel and rootfs paths must be absolute"
  [[ "$BASE_ROOTFS" != "$ROOTFS" ]] \
    || die "base rootfs and private rootfs must be different files"
}

check_source() {
  validate_paths
  local required=(
    README.md guest_server.rs client.rs prepare.sh run-example.sh
    service.toml tcp-startup-failure.toml
    http-vm-204.toml http-vm-302.toml http-vm-404.toml http-vm-503.toml
    http-exec-204.toml http-exec-302.toml http-exec-404.toml http-exec-503.toml
    readiness-recovery.toml liveness-restart.toml
    zero-probes.toml zero-probes-failure.toml
    client-healthy.toml client-tcp-failure.toml
    client-readiness-before.toml client-readiness-during.toml
    client-readiness-after.toml client-zero-probes.toml
    client-zero-probes-failure.toml
  )
  local item
  for item in "${required[@]}"; do
    [[ -f "$EXAMPLE_DIR/$item" ]] \
      || die "missing checked-in source: $EXAMPLE_DIR/$item"
  done

  local client_specs=(
    client-healthy.toml client-tcp-failure.toml
    client-readiness-before.toml client-readiness-during.toml
    client-readiness-after.toml client-zero-probes.toml
    client-zero-probes-failure.toml
  )
  local spec
  for spec in "${client_specs[@]}"; do
    spec="$EXAMPLE_DIR/$spec"
    [[ "$(grep -Fxc '[job]' "$spec")" -eq 1 ]] \
      || die "peer client must declare exactly one [job]: $spec"
    [[ "$(grep -Fxc '[vm]' "$spec")" -eq 1 ]] \
      || die "peer client must declare exactly one [vm]: $spec"
    ! grep -Fxq '[exec]' "$spec" \
      || die "peer client must not declare [exec]: $spec"
    grep -Fq "command = \"$GUEST_CLIENT\"" "$spec" \
      || die "peer client guest command differs from preparation contract: $spec"
    grep -Fq "kernel = \"$KERNEL\"" "$spec" \
      || die "peer client kernel differs from preparation contract: $spec"
    grep -Fq "rootfs = \"$ROOTFS\"" "$spec" \
      || die "peer client rootfs differs from preparation contract: $spec"
  done

  local vm_service_specs=(
    service.toml tcp-startup-failure.toml
    http-vm-204.toml http-vm-302.toml http-vm-404.toml http-vm-503.toml
    readiness-recovery.toml liveness-restart.toml
    zero-probes.toml zero-probes-failure.toml
  )
  for spec in "${vm_service_specs[@]}"; do
    spec="$EXAMPLE_DIR/$spec"
    [[ "$(grep -Fxc '[service]' "$spec")" -eq 1 ]] \
      || die "VM Service fixture must declare exactly one [service]: $spec"
    [[ "$(grep -Fxc '[vm]' "$spec")" -eq 1 ]] \
      || die "VM Service fixture must declare exactly one [vm]: $spec"
    ! grep -Fxq '[exec]' "$spec" \
      || die "VM Service fixture must not declare [exec]: $spec"
    grep -Fq "command = \"$GUEST_SERVER\"" "$spec" \
      || die "VM Service guest command differs from preparation contract: $spec"
    grep -Fq "kernel = \"$KERNEL\"" "$spec" \
      || die "VM Service kernel differs from preparation contract: $spec"
    grep -Fq "rootfs = \"$ROOTFS\"" "$spec" \
      || die "VM Service rootfs differs from preparation contract: $spec"
  done

  local exec_control_specs=(
    http-exec-204.toml http-exec-302.toml
    http-exec-404.toml http-exec-503.toml
  )
  for spec in "${exec_control_specs[@]}"; do
    spec="$EXAMPLE_DIR/$spec"
    [[ "$(grep -Fxc '[service]' "$spec")" -eq 1 ]] \
      || die "E10 control must declare exactly one [service]: $spec"
    [[ "$(grep -Fxc '[exec]' "$spec")" -eq 1 ]] \
      || die "E10 control must retain exactly one [exec]: $spec"
    ! grep -Fxq '[vm]' "$spec" \
      || die "E10 Exec control must not declare [vm]: $spec"
    ! grep -Fxq '[job]' "$spec" \
      || die "E10 Exec control is a Service, not a peer client Job: $spec"
    grep -Fq "command = \"$SERVER\"" "$spec" \
      || die "E10 Exec control command differs from preparation contract: $spec"
  done

  grep -Fq 'SVM-E08-GUEST-OK' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server reply sentinel is absent"
  grep -Fq 'SVM-E08-GUEST-OK' "$EXAMPLE_DIR/client.rs" \
    || die "VM client reply oracle is absent"
  grep -Fq 'SVM-E10-FAILURE-BODY-MUST-NOT-LEAK' "$EXAMPLE_DIR/guest_server.rs" \
    || die "E10 failure-body sentinel is absent"
  grep -Fq 'if status == 503' "$EXAMPLE_DIR/guest_server.rs" \
    || die "E10 failure-body fixture is not attached to a failing response"
  [[ "$(grep -Fc 'FAILURE_DIAGNOSTIC_SENTINEL' "$EXAMPLE_DIR/guest_server.rs")" -ge 2 ]] \
    || die "E10 failure-body sentinel is not wired into a failing response"
  grep -Fq 'service-vm-e08.svc.overdrive.local' \
    "$EXAMPLE_DIR/client-healthy.toml" \
    || die "healthy VM client does not resolve the checked-in Service name"
  grep -Fq 'ready_sequence: vec![204]' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server default readiness fixture is absent"
  grep -Fq -- '--liveness-fail-after-ms' "$EXAMPLE_DIR/guest_server.rs" \
    || die "guest server liveness fixture is absent"
  if grep -Eq '^\[\[?health_check' \
    "$EXAMPLE_DIR/zero-probes.toml" "$EXAMPLE_DIR/zero-probes-failure.toml"; then
    die "zero-probe specs declare a health_check table"
  fi
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

remove_owned_output() {
  [[ -e "$OUTPUT_ROOT" ]] || return 0
  [[ -f "$OWNERSHIP_MARKER" ]] \
    || die "refusing to remove unmarked path: $OUTPUT_ROOT"
  local marker
  marker="$(<"$OWNERSHIP_MARKER")"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$marker" == "svm-e08-owned-v1:$OWNERSHIP_TOKEN" ]] \
      || die "refusing to remove output owned by another invocation: $OUTPUT_ROOT"
  else
    case "$marker" in
      svm-e08-owned-v1|svm-e08-owned-v1:*) ;;
      *) die "refusing to remove path with an unknown ownership marker: $OUTPUT_ROOT" ;;
    esac
  fi
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
    echo "svm-e08 prepare: bounded mount/loop cleanup failed; leaving process-owned output for inspection" >&2
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

verify_marker() {
  [[ -f "$OWNERSHIP_MARKER" ]] || die "missing ownership marker: $OWNERSHIP_MARKER"
  local marker
  marker="$(<"$OWNERSHIP_MARKER")"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$marker" == "svm-e08-owned-v1:$OWNERSHIP_TOKEN" ]] \
      || die "materialization belongs to another invocation"
  else
    case "$marker" in
      svm-e08-owned-v1|svm-e08-owned-v1:*) ;;
      *) die "unexpected ownership marker content" ;;
    esac
  fi
}

verify_materialization_without_mount() {
  verify_marker
  [[ -r "$KERNEL" && -f "$ROOTFS" ]] || die "private kernel/rootfs is incomplete"
  verify_static_binary "$SERVER"
  verify_static_binary "$CLIENT"
  [[ "$(stat -c %d "$ROOTFS")" == "$(stat -c %d "$DATA_DIR")" ]] \
    || die "private rootfs and serve data directory are on different filesystems"
  [[ "$(stat -c %a "$OUTPUT_ROOT")" == "711" ]] \
    || die "output root must be mode 0711 for confined-VMM traversal"
  [[ "$(stat -c %a "$DATA_DIR")" == "711" ]] \
    || die "serve data directory must be mode 0711 for confined-VMM traversal"
  setpriv --reuid=4200 --regid=4200 --clear-groups test -x "$DATA_DIR" \
    || die "uid 4200 cannot traverse the serve data path and its ancestors"
  [[ "$(stat -c %a "$CREDS_DIR")" == "700" ]] \
    || die "credential directory must be mode 0700"
  [[ "$(stat -c %a "$KEK_FILE")" == "400" ]] \
    || die "KEK credential must be mode 0400"
  [[ "$(stat -c %s "$KEK_FILE")" == "32" ]] \
    || die "KEK credential must contain exactly 32 raw bytes"
}

verify_guest_binaries() {
  verify_static_binary "$MOUNT_DIR$GUEST_SERVER"
  verify_static_binary "$MOUNT_DIR$GUEST_CLIENT"
}

prepare() {
  require_native_linux
  check_source
  local command
  for command in chmod cp dirname file grep head install losetup mount mountpoint \
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
    printf 'svm-e08-owned-v1:%s\n' "$OWNERSHIP_TOKEN" >"$OWNERSHIP_MARKER"
  else
    printf '%s\n' 'svm-e08-owned-v1' >"$OWNERSHIP_MARKER"
  fi
  install -d -m 0755 "$MOUNT_DIR"
  install -d -m 0711 "$DATA_DIR"
  install -d -m 0700 "$CONFIG_DIR" "$CREDS_DIR"

  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/guest_server.rs" -o "$SERVER"
  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/client.rs" -o "$CLIENT"
  chmod 0755 "$SERVER" "$CLIENT"
  verify_static_binary "$SERVER"
  verify_static_binary "$CLIENT"

  cp --reflink=auto --sparse=auto "$BASE_KERNEL" "$KERNEL"
  chmod 0644 "$KERNEL"
  cp --reflink=always --sparse=auto "$BASE_ROOTFS" "$ROOTFS"
  chmod 0600 "$ROOTFS"
  [[ "$(stat -c %d "$ROOTFS")" == "$(stat -c %d "$DATA_DIR")" ]] \
    || die "reflink staging and serve data directory must share a filesystem"

  mount_private_rootfs
  install -d -m 0755 "$MOUNT_DIR$(dirname "$GUEST_SERVER")"
  install -m 0755 "$SERVER" "$MOUNT_DIR$GUEST_SERVER"
  install -m 0755 "$CLIENT" "$MOUNT_DIR$GUEST_CLIENT"
  verify_guest_binaries
  sync "$MOUNT_DIR$GUEST_SERVER" "$MOUNT_DIR$GUEST_CLIENT"
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"

  head -c 32 /dev/urandom >"$KEK_FILE"
  chmod 0400 "$KEK_FILE"
  verify_materialization_without_mount
  PREPARE_COMMITTED=1
  PREPARE_OWNS_OUTPUT=0
  trap - EXIT HUP INT TERM
  echo "prepared SVM E08 runtime materialization at $OUTPUT_ROOT"
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
  verify_guest_binaries
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"
  PREPARE_COMMITTED=1
  trap - EXIT HUP INT TERM
  echo "verified SVM E08 runtime materialization at $OUTPUT_ROOT"
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
  echo "removed SVM E08-owned materialization from $OUTPUT_ROOT"
}

case "${1:-}" in
  check-source) check_source ;;
  prepare) prepare ;;
  check) check ;;
  cleanup) cleanup ;;
  *) usage ;;
esac
