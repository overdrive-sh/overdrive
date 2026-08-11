#!/usr/bin/env bash
# PROBE increment-d (P6) — build the P6 guest init + host collector and a rootfs
# that carries the virtiofs module.
#
# REUSES increment-a: the unwrapped arm64 `Image` and the vsock modules are
# taken from /var/tmp/spike-increment-a. Only the init binary and one extra
# kernel module differ, so P6 is measured against the SAME booting VM P1/P2
# validated — not a different one.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-d
STAGE="$OUT/rootfs"
TARGET=aarch64-unknown-linux-musl
KVER=${KVER:-$(uname -r)}

export RUSTUP_HOME=${RUSTUP_HOME:-/home/marcus.guest/.rustup}
export CARGO_HOME=${CARGO_HOME:-/home/marcus.guest/.cargo}
export CARGO_TARGET_DIR="$HERE/target"

echo "=== [build] reusing increment-a kernel image (no rebuild)"
[ -f "$A_OUT/Image" ] || { echo "!!! missing $A_OUT/Image — run increment-a/build.sh first"; exit 1; }
file -b "$A_OUT/Image"

mkdir -p "$OUT"
find "$HERE/probe/src" -name '*.rs' -exec touch {} +

echo
echo "=== [build] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
file "$BIN_DIR/guest-init-fs" "$BIN_DIR/host-collector"

echo
echo "=== [build] staging rootfs tree (native fs — virtiofs refuses mknod)"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt,modules}
install -m 0755 "$BIN_DIR/guest-init-fs" "$STAGE/init"

echo "=== [build] kernel modules"
grep -E 'CONFIG_(FUSE_FS|VIRTIO_FS|VSOCKETS|VIRTIO_VSOCKETS)=' "/boot/config-$KVER" || true
# CONFIG_FUSE_FS=y (built in) but CONFIG_VIRTIO_FS=m, so virtiofs.ko must ship
# in the rootfs alongside the three vsock modules increment-a already needed.
for spec in \
  "net/vmw_vsock/vsock" \
  "net/vmw_vsock/vmw_vsock_virtio_transport_common" \
  "net/vmw_vsock/vmw_vsock_virtio_transport" \
  "fs/fuse/virtiofs"
do
  name="$(basename "$spec")"
  src="/lib/modules/$KVER/kernel/$spec.ko.zst"
  [ -f "$src" ] || { echo "!!! missing module $src"; exit 1; }
  zstd -d -f -q "$src" -o "$STAGE/modules/$name.ko"
  echo "    staged $name.ko ($(stat -c %s "$STAGE/modules/$name.ko") bytes)"
done

mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"
chmod 666 "$STAGE/dev/null"

echo
echo "=== [build] mkfs.ext4 -d (no loop mount, no root needed for population)"
rm -f "$OUT/rootfs.ext4"
truncate -s 96M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-d -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4"
debugfs -R "ls -l /modules" "$OUT/rootfs.ext4" 2>/dev/null || true
echo "=== [build] DONE"
