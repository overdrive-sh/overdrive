#!/usr/bin/env bash
# PROBE increment-k — build the ondemand-restore guest and the host harness.
# Reuses increment-a's CH-loadable kernel, same as e/f/g/i/j.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-k
STAGE="$OUT/rootfs"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"

_HOME="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
export RUSTUP_HOME="${RUSTUP_HOME:-${_HOME}/.rustup}"
export CARGO_HOME="${CARGO_HOME:-${_HOME}/.cargo}"
export PATH="${CARGO_HOME}/bin:${PATH}"
export CARGO_TARGET_DIR="$HERE/target"

[ -f "$A_OUT/kernel" ] || { echo "!!! run increment-a/build.sh first" >&2; exit 1; }
mkdir -p "$OUT"
install -m 0644 "$A_OUT/kernel" "$OUT/kernel"   # 0644: CH runs uid-dropped

find "$HERE/probe/src" -name '*.rs' -exec touch {} +

# The GUEST is musl-static (it is PID 1 in a rootfs with no libc).
echo "=== [build] guest: cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" \
  --manifest-path "$HERE/probe/Cargo.toml" --bin guest-init-ondemand
GBIN="$CARGO_TARGET_DIR/$TARGET/release/guest-init-ondemand"
file "$GBIN"

# The HOST harness is an ordinary host binary; no musl needed.
echo "=== [build] host: cargo build --release"
cargo build --release --manifest-path "$HERE/probe/Cargo.toml" --bin host-ondemand
HBIN="$CARGO_TARGET_DIR/release/host-ondemand"
file "$HBIN"
install -m 0755 "$HBIN" "$OUT/host-ondemand"

echo "=== [build] staging rootfs"
rm -rf "$STAGE"; mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt}
install -m 0755 "$GBIN" "$STAGE/init"
mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"; chmod 666 "$STAGE/dev/null"

rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-k -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4" "$OUT/kernel" "$OUT/host-ondemand"
echo "=== [build] DONE"
