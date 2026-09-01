#!/usr/bin/env bash
# Materialize the checked-in E07 example on the qualified native-metal host.
# Source and workload specs remain checked in; this script creates only runtime
# binaries, a private appliance image, isolated serve state, and one KEK file.
set -euo pipefail

EXAMPLE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly EXAMPLE_DIR
readonly STAGING_ROOT="/srv/vm/overdrive-testing"
readonly OUTPUT_ROOT="$STAGING_ROOT/gti-e07"
readonly OWNERSHIP_MARKER="$OUTPUT_ROOT/.gti-e07-owned"
readonly BASE_KERNEL="${GTI_E07_BASE_KERNEL:-$STAGING_ROOT/kernel}"
readonly BASE_ROOTFS="${GTI_E07_BASE_ROOTFS:-$STAGING_ROOT/rootfs.ext4}"
readonly KERNEL="$OUTPUT_ROOT/kernel"
readonly ROOTFS="$OUTPUT_ROOT/rootfs.ext4"
readonly BIN_DIR="$OUTPUT_ROOT/bin"
readonly CALLEE="$BIN_DIR/e07-callee"
readonly CALLER="$BIN_DIR/e07-caller"
readonly MOUNT_DIR="$OUTPUT_ROOT/mnt"
readonly DATA_DIR="$OUTPUT_ROOT/data"
readonly CONFIG_DIR="$OUTPUT_ROOT/config"
readonly CREDS_DIR="$OUTPUT_ROOT/credentials"
readonly KEK_FILE="$CREDS_DIR/overdrive-ca-root"
readonly GUEST_CALLER="/opt/overdrive/examples/gti/e07-caller"
readonly STATIC_TARGET="x86_64-unknown-linux-musl"
readonly OWNERSHIP_TOKEN="${GTI_E07_OWNERSHIP_TOKEN:-}"

LOOP_DEVICE=""
PREPARE_COMMITTED=0
PREPARE_OWNS_OUTPUT=0

die() {
  echo "gti-e07 prepare: $*" >&2
  exit 1
}

usage() {
  cat >&2 <<'USAGE'
usage: prepare.sh check-source|prepare|check|cleanup

  check-source  validate only the checked-in sources/spec paths (host-safe)
  prepare       materialize the fixed native-metal runtime paths
  check         verify an existing materialization, including the guest image
  cleanup       remove only the marker-owned fixed materialization

Optional base-image overrides:
  GTI_E07_BASE_KERNEL=/absolute/path/to/kernel
  GTI_E07_BASE_ROOTFS=/absolute/path/to/rootfs.ext4
USAGE
  exit 2
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || die "required command is unavailable: $1"
}

validate_paths() {
  [[ "$STAGING_ROOT" = /* ]] || die "staging root must be absolute"
  [[ "$STAGING_ROOT" != "/" ]] || die "refusing to use / as the staging root"
  [[ "$OUTPUT_ROOT" == "$STAGING_ROOT/gti-e07" ]] \
    || die "internal output-root invariant failed"
  [[ "$BASE_KERNEL" = /* && "$BASE_ROOTFS" = /* ]] \
    || die "base kernel and rootfs paths must be absolute"
  [[ "$BASE_ROOTFS" != "$ROOTFS" ]] \
    || die "base rootfs and private rootfs must be different files"
}

check_source() {
  validate_paths
  local required=(
    README.md caller.rs callee.rs caller.toml callee.toml prepare.sh
    run-example.sh session-lifecycle.sh session-wrapper.sh
  )
  local item
  for item in "${required[@]}"; do
    [[ -f "$EXAMPLE_DIR/$item" ]] || die "missing checked-in source: $EXAMPLE_DIR/$item"
  done

  grep -Fq "kernel = \"$KERNEL\"" "$EXAMPLE_DIR/caller.toml" \
    || die "caller.toml kernel path differs from the preparation contract"
  grep -Fq "rootfs = \"$ROOTFS\"" "$EXAMPLE_DIR/caller.toml" \
    || die "caller.toml rootfs path differs from the preparation contract"
  grep -Fq "command = \"$CALLEE\"" "$EXAMPLE_DIR/callee.toml" \
    || die "callee.toml command path differs from the preparation contract"
  grep -Fq 'command = "/opt/overdrive/examples/gti/e07-caller"' \
    "$EXAMPLE_DIR/caller.toml" \
    || die "caller.toml guest command differs from the preparation contract"
  grep -Fq 'startup = []' "$EXAMPLE_DIR/callee.toml" \
    || die "callee.toml must explicitly opt out of the unreachable inferred startup probe"
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
    [[ "$marker" == "gti-e07-owned-v1:$OWNERSHIP_TOKEN" ]] \
      || die "refusing to remove output owned by another invocation: $OUTPUT_ROOT"
  else
    case "$marker" in
      gti-e07-owned-v1|gti-e07-owned-v1:*) ;;
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
    echo "gti-e07 prepare: bounded mount/loop cleanup failed; leaving process-owned output for inspection" >&2
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
  [[ -f "$OWNERSHIP_MARKER" ]] || die "missing ownership marker: $OWNERSHIP_MARKER"
  local marker
  marker="$(<"$OWNERSHIP_MARKER")"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$marker" == "gti-e07-owned-v1:$OWNERSHIP_TOKEN" ]] \
      || die "materialization belongs to another invocation"
  else
    case "$marker" in
      gti-e07-owned-v1|gti-e07-owned-v1:*) ;;
      *) die "unexpected ownership marker content" ;;
    esac
  fi
  [[ -r "$KERNEL" && -f "$ROOTFS" ]] || die "private kernel/rootfs is incomplete"
  verify_static_binary "$CALLEE"
  verify_static_binary "$CALLER"
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

prepare() {
  require_native_linux
  check_source
  local command
  for command in cp file grep install losetup mount mountpoint readelf rustc setpriv stat timeout umount; do
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

  # Arm teardown and record process-local ownership before the first write to
  # the fixed output tree. A signal between mkdir and durable marker creation
  # can therefore remove only the tree this invocation established; the
  # explicit cleanup command continues to require the durable marker.
  trap on_prepare_exit EXIT
  trap 'exit 130' HUP INT TERM
  PREPARE_OWNS_OUTPUT=1
  install -d -m 0711 "$OUTPUT_ROOT"
  if [[ -n "$OWNERSHIP_TOKEN" ]]; then
    [[ "$OWNERSHIP_TOKEN" =~ ^[A-Za-z0-9._-]+$ ]] \
      || die "ownership token contains unsupported characters"
    printf 'gti-e07-owned-v1:%s\n' "$OWNERSHIP_TOKEN" >"$OWNERSHIP_MARKER"
  else
    printf '%s\n' 'gti-e07-owned-v1' >"$OWNERSHIP_MARKER"
  fi
  install -d -m 0755 "$BIN_DIR" "$MOUNT_DIR"
  install -d -m 0711 "$DATA_DIR"
  install -d -m 0700 "$CONFIG_DIR" "$CREDS_DIR"

  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/callee.rs" -o "$CALLEE"
  rustc --edition=2024 -D warnings -C opt-level=2 -C strip=symbols \
    --target "$STATIC_TARGET" "$EXAMPLE_DIR/caller.rs" -o "$CALLER"
  chmod 0755 "$CALLEE" "$CALLER"
  verify_static_binary "$CALLEE"
  verify_static_binary "$CALLER"

  cp --reflink=auto --sparse=auto "$BASE_KERNEL" "$KERNEL"
  chmod 0644 "$KERNEL"
  cp --reflink=always --sparse=auto "$BASE_ROOTFS" "$ROOTFS"
  chmod 0600 "$ROOTFS"
  [[ "$(stat -c %d "$ROOTFS")" == "$(stat -c %d "$DATA_DIR")" ]] \
    || die "reflink staging and serve data directory must share a filesystem"

  mount_private_rootfs
  install -d -m 0755 "$MOUNT_DIR$(dirname "$GUEST_CALLER")"
  install -m 0755 "$CALLER" "$MOUNT_DIR$GUEST_CALLER"
  verify_static_binary "$MOUNT_DIR$GUEST_CALLER"
  sync "$MOUNT_DIR$GUEST_CALLER"
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"

  head -c 32 /dev/urandom >"$KEK_FILE"
  chmod 0400 "$KEK_FILE"
  verify_materialization_without_mount
  PREPARE_COMMITTED=1
  PREPARE_OWNS_OUTPUT=0
  trap - EXIT HUP INT TERM
  echo "prepared E07 runtime materialization at $OUTPUT_ROOT"
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
  verify_static_binary "$MOUNT_DIR$GUEST_CALLER"
  unmount_private_rootfs || die "bounded rootfs unmount/loop detach failed"
  PREPARE_COMMITTED=1
  trap - EXIT HUP INT TERM
  echo "verified E07 runtime materialization at $OUTPUT_ROOT"
}

cleanup() {
  require_native_linux
  require_command losetup
  require_command mountpoint
  require_command timeout
  require_command umount
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
  echo "removed E07-owned materialization from $OUTPUT_ROOT"
}

case "${1:-}" in
  check-source) check_source ;;
  prepare) prepare ;;
  check) check ;;
  cleanup) cleanup ;;
  *) usage ;;
esac
