#!/usr/bin/env bash
# PROBE increment-g — build the snapshot/restore guest and its rootfs.
# Reuses increment-a's CH-loadable kernel, same as e and f.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-g
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
echo "=== [build] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
file "$BIN_DIR/guest-init-snap"

echo "=== [build] staging rootfs"
rm -rf "$STAGE"; mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt,modules}
install -m 0755 "$BIN_DIR/guest-init-snap" "$STAGE/init"
# ext4 and virtio_blk are CONFIG_*=y, but CONFIG_VIRTIO_FS=m — so the S-2 `fs`
# arm needs virtiofs.ko shipped in the rootfs. Omitting it made the virtiofs
# mount fail with ENODEV and the probe reported `vol=none`, which looked like a
# result and was actually a harness gap.
KVER="${KVER:-$(uname -r)}"
zstd -d -f -q "/lib/modules/$KVER/kernel/fs/fuse/virtiofs.ko.zst" -o "$STAGE/modules/virtiofs.ko"
echo "=== [build] staged virtiofs.ko ($(stat -c %s "$STAGE/modules/virtiofs.ko") bytes)"
mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"; chmod 666 "$STAGE/dev/null"

rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-g -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4"
echo "=== [build] DONE"
