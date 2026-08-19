#!/usr/bin/env bash
# PROBE increment-l — build the memory-as-cache guest, the harness, the rootfs,
# and the BLANK volume image.
#
# Reuses increment-a's CH-loadable kernel, same as e/f/g/h/i/j/k.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-l
STAGE="$OUT/rootfs"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"

_HOME="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
export RUSTUP_HOME="${RUSTUP_HOME:-${_HOME}/.rustup}"
export CARGO_HOME="${CARGO_HOME:-${_HOME}/.cargo}"
export PATH="${CARGO_HOME}/bin:${PATH}"
export CARGO_TARGET_DIR="$HERE/target"
# There is no rustup DEFAULT toolchain configured for this user, so every bare
# `cargo` resolves to an error. Pin the one installed toolchain explicitly
# rather than mutating the shared box's rustup state.
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-$(ls "$RUSTUP_HOME/toolchains" | head -1)}"

[ -f "$A_OUT/kernel" ] || { echo "!!! run increment-a/build.sh first" >&2; exit 1; }
mkdir -p "$OUT"
install -m 0644 "$A_OUT/kernel" "$OUT/kernel"   # 0644: CH may run uid-dropped

find "$HERE/probe/src" -name '*.rs' -exec touch {} +
echo "=== [build] toolchain $RUSTUP_TOOLCHAIN  target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
file "$BIN_DIR/guest-init-l"
install -m 0755 "$BIN_DIR/host-cache" "$OUT/host-cache"

echo "=== [build] staging rootfs"
rm -rf "$STAGE"; mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt/vol}
install -m 0755 "$BIN_DIR/guest-init-l" "$STAGE/init"
mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"; chmod 666 "$STAGE/dev/null"

rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-l -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null

# The BLANK volume. Default mkfs options on purpose: has_journal, data=ordered.
# Anything exotic here would make the durability result a property of the probe's
# mkfs flags rather than of the model under test.
rm -f "$OUT/vol-blank.ext4"
truncate -s 256M "$OUT/vol-blank.ext4"
mkfs.ext4 -F -L spikevol-l "$OUT/vol-blank.ext4" >/dev/null
dumpe2fs -h "$OUT/vol-blank.ext4" 2>/dev/null | grep -E 'Filesystem (state|features)'

ls -la "$OUT/"
echo "=== [build] DONE"
