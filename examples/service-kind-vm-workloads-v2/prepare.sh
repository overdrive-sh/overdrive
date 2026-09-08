#!/usr/bin/env bash
# Prepare the reusable, versioned E09-v2 guest artifacts.
#
# This script owns only /srv/vm/overdrive-testing/svm-e08-v2 (or the explicit
# SVM_E09_V2_OUTPUT_ROOT override).  The guest programs and their source
# specifications remain in the original checked-in E08 bundle; this versioned
# materialization keeps its mutable rootfs, credentials, and serve state
# separate from that bundle.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
SOURCE_DIR="$(cd "$EXAMPLE_DIR/../service-kind-vm-workloads" && pwd)"
readonly SOURCE_DIR
readonly STAGING_ROOT="/srv/vm/overdrive-testing"
readonly OUTPUT_ROOT="${SVM_E09_V2_OUTPUT_ROOT:-$STAGING_ROOT/svm-e08-v2}"
readonly OWNERSHIP_MARKER="$OUTPUT_ROOT/.svm-e09-v2-owned"
readonly BASE_KERNEL="${SVM_E09_V2_BASE_KERNEL:-$STAGING_ROOT/kernel}"
readonly BASE_ROOTFS="${SVM_E09_V2_BASE_ROOTFS:-$STAGING_ROOT/rootfs.ext4}"
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
readonly OWNERSHIP_TOKEN="${SVM_E09_V2_OWNERSHIP_TOKEN:-}"

LOOP_DEVICE=""
PREPARE_COMMITTED=0
PREPARE_OWNS_OUTPUT=0

die() {
  echo "svm-e09-v2 prepare: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
usage: prepare.sh check-source|prepare|check|cleanup

  check-source  validate the unchanged checked-in E08 source (host-safe)
  prepare       compile and materialize the versioned reusable artifacts
  check         verify the materialization, including guest binaries
  cleanup       remove only this script's marker-owned materialization

Optional overrides:
  SVM_E09_V2_OUTPUT_ROOT=/absolute/path/below/staging-root
  SVM_E09_V2_BASE_KERNEL=/absolute/path/to/kernel
  SVM_E09_V2_BASE_ROOTFS=/absolute/path/to/rootfs.ext4
USAGE
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

validate_paths() {
  [[ "$STAGING_ROOT" != "/" ]] || die "refusing to use / as the staging root"
  [[ "$OUTPUT_ROOT" = /* && "$OUTPUT_ROOT" == "$STAGING_ROOT"/* ]] \
    || die "output root must be an absolute path below $STAGING_ROOT"
  [[ "$OUTPUT_ROOT" != "$STAGING_ROOT" ]] \
    || die "refusing to use the staging root itself as output"
  [[ "$OUTPUT_ROOT" =~ ^/srv/vm/overdrive-testing/[A-Za-z0-9._/-]+$ ]] \
    || die "output root may contain only safe path characters below $STAGING_ROOT"
  case "$OUTPUT_ROOT" in
    */../*|*/..|*/./*|*/.)
      die "output root may not contain path traversal components" ;;
  esac
  [[ "$OUTPUT_ROOT" != *'//'* ]] \
    || die "output root may not contain repeated path separators"
  [[ "$BASE_KERNEL" = /* && "$BASE_ROOTFS" = /* ]] \
    || die "base kernel and rootfs paths must be absolute"
  [[ "$BASE_ROOTFS" != "$ROOTFS" ]] \
    || die "base rootfs and private rootfs must be different files"
  [[ "$OUTPUT_ROOT" != *'|'* ]] \
    || die "output root may not contain the spec materialization delimiter"
}

check_source() {
  validate_paths
  # The original source checker is the SSOT for the accepted guest programs,
  # client/job shape, and E08/E09 fixture inputs. It performs no writes. Clear
  # its v1 output override so this check cannot accidentally inspect v2 state.
  env -u SVM_E08_OUTPUT_ROOT -u SVM_E08_BASE_KERNEL -u SVM_E08_BASE_ROOTFS \
    "$SOURCE_DIR/prepare.sh" check-source
  local item
  for item in guest_server.rs client.rs service.toml tcp-startup-failure.toml \
    client-healthy.toml client-tcp-failure.toml; do
    [[ -f "$SOURCE_DIR/$item" ]] || die "missing checked-in source: $SOURCE_DIR/$item"
  done
  grep -Fq 'SVM-E08-GUEST-OK' "$SOURCE_DIR/guest_server.rs" \
    || die "guest reply sentinel is absent from the checked-in server"
  grep -Fq 'SVM-E08-GUEST-OK' "$SOURCE_DIR/client.rs" \
    || die "guest reply oracle is absent from the checked-in client"
  echo "checked checked-in E08 sources for E09-v2"
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
    [[ "$marker" == "svm-e09-v2-owned-v1:$OWNERSHIP_TOKEN" ]] \
      || die "materialization belongs to another invocation"
  else
    case "$marker" in
      svm-e09-v2-owned-v1|svm-e09-v2-owned-v1:*) ;;
      *) die "unexpected ownership marker content" ;;
    esac
  fi
}

remove_owned_output() {
  [[ -e "$OUTPUT_ROOT" ]] || return 0
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
    die "refusing to remove process-owned output while $MOUNT_DIR is mounted"
  fi
  rm -rf -- "$OUTPUT_ROOT"
}

on_prepare_exit() {
  local rc=$?
  trap - EXIT HUP INT TERM
  if ! unmount_private_rootfs; then
    echo "svm-e09-v2 prepare: bounded mount/loop cleanup failed; output left for inspection" >&2
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

verify_guest_binaries() {
  verify_static_binary "$MOUNT_DIR$GUEST_SERVER"
  verify_static_binary "$MOUNT_DIR$GUEST_CLIENT"
}

verify_materialization_without_mount() {
  verify_marker
  [[ -r "$KERNEL" && -f "$ROOTFS" ]] || die "private kernel/rootfs is incomplete"
  verify_static_binary "$SERVER"
  verify_static_binary "$CLIENT"
  [[ "$(stat -c %d "$ROOTFS")" == "$(stat -c %d "$DATA_DIR")" ]] \
    || die "private rootfs and serve data must share a filesystem"
  [[ "$(stat -c %a "$OUTPUT_ROOT")" == "711" ]] \
    || die "output root must be mode 0711 for confined-VMM traversal"
  [[ "$(stat -c %a "$DATA_DIR")" == "711" ]] \
    || die "serve data directory must be mode 0711 for confined-VMM traversal"
  setpriv --reuid=4200 --regid=4200 --clear-groups test -x "$DATA_DIR" \
    || die "uid 4200 cannot traverse the serve data path"
  [[ "$(stat -c %a "$CREDS_DIR")" == "700" ]] \
    || die "credential directory must be mode 0700"
  [[ "$(stat -c %a "$KEK_FILE")" == "400" ]] \
    || die "KEK credential must be mode 0400"
  [[ "$(stat -c %s "$KEK_FILE")" == "32" ]] \
    || die "KEK credential must contain exactly 32 raw bytes"
}

prepare() {
  require_native_linux
  check_source
  local command
  for command in chmod cp dirname file grep head install losetup mount mountpoint \
    readelf rm rustc setpriv stat sync timeout umount; do
    require_command "$command"
  done
  [[ -r "$BASE_KERNEL" && -f "$BASE_KERNEL" ]] \
    || die "base kernel is absent or unreadable: $BASE_KERNEL"
  [[ -r "$BASE_ROOTFS" && -f "$BASE_ROOTFS" ]] \
    || die "base rootfs is absent or unreadable: $BASE_ROOTFS"
  [[ ! -e "$OUTPUT_ROOT" ]] \
    || die "$OUTPUT_ROOT already exists; run prepare.sh cleanup first"
  rustc --print target-libdir --target "$STATIC_TARGET" >/dev/null 2>&1 \
    || die "Rust target $STATIC_TARGET is unavailable"

  trap on_prepare_exit EXIT
  trap 'exit 130' HUP INT TERM
  PREPARE_OWNS_OUTPUT=1
  install -d -m 0711 "$OUTPUT_ROOT"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$OWNERSHIP_TOKEN" =~ ^[A-Za-z0-9._-]+$ ]] \
      || die "ownership token contains unsupported characters"
    printf 'svm-e09-v2-owned-v1:%s\n' "$OWNERSHIP_TOKEN" >"$OWNERSHIP_MARKER"
  else
    printf '%s\n' 'svm-e09-v2-owned-v1' >"$OWNERSHIP_MARKER"
  fi
  install -d -m 0755 "$MOUNT_DIR"
  install -d -m 0711 "$DATA_DIR"
  install -d -m 0700 "$CONFIG_DIR" "$CREDS_DIR"

  # Exactly one compilation of each immutable checked-in guest program for the
  # suite. Every allocation later receives a product-owned FICLONE rootfs.
  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$SOURCE_DIR/guest_server.rs" -o "$SERVER"
  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$SOURCE_DIR/client.rs" -o "$CLIENT"
  chmod 0755 "$SERVER" "$CLIENT"
  verify_static_binary "$SERVER"
  verify_static_binary "$CLIENT"

  cp --reflink=auto --sparse=auto "$BASE_KERNEL" "$KERNEL"
  chmod 0644 "$KERNEL"
  cp --reflink=always --sparse=auto "$BASE_ROOTFS" "$ROOTFS"
  chmod 0600 "$ROOTFS"
  [[ "$(stat -c %d "$ROOTFS")" == "$(stat -c %d "$DATA_DIR")" ]] \
    || die "private rootfs and serve data must share a filesystem"

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
  echo "prepared SVM E09-v2 reusable artifacts at $OUTPUT_ROOT"
}

check() {
  require_native_linux
  check_source
  local command
  for command in file losetup mount mountpoint readelf setpriv stat timeout umount; do
    require_command "$command"
  done
  verify_materialization_without_mount
  trap on_prepare_exit EXIT
  trap 'exit 130' HUP INT TERM
  mount_private_rootfs
  verify_guest_binaries
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"
  PREPARE_COMMITTED=1
  trap - EXIT HUP INT TERM
  echo "verified SVM E09-v2 reusable artifacts at $OUTPUT_ROOT"
}

cleanup() {
  require_native_linux
  validate_paths
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
  echo "removed SVM E09-v2-owned materialization from $OUTPUT_ROOT"
}

case "${1:-}" in
  check-source) check_source ;;
  prepare) prepare ;;
  check) check ;;
  cleanup) cleanup ;;
  *) usage ;;
esac
