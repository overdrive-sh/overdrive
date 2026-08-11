#!/usr/bin/env bash
# PROBE increment-e (P6 on BARE METAL) — build the P6 guest init + host
# collector and a rootfs that carries the virtiofs module.
#
# increment-e is increment-d ported to env B (bare-metal x86_64). increment-d
# is left untouched as the env-A record.
#
# REUSES increment-a: the CH-loadable kernel and the vsock modules are taken
# from /var/tmp/spike-increment-a, so P6 is measured against the SAME booting VM
# P1/P2 validated on this box — not a different one. Run increment-a/build.sh
# first.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-e
STAGE="$OUT/rootfs"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"
KVER="${KVER:-$(uname -r)}"

# Derive from the invoking user rather than hardcoding an account — increment-d
# hardcoded Lima's `marcus.guest`, which does not exist on the metal box.
_HOME="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
export RUSTUP_HOME="${RUSTUP_HOME:-${_HOME}/.rustup}"
export CARGO_HOME="${CARGO_HOME:-${_HOME}/.cargo}"
export PATH="${CARGO_HOME}/bin:${PATH}"
export CARGO_TARGET_DIR="$HERE/target"

echo "=== [build] arch=$ARCH target=$TARGET kver=$KVER"

# increment-a's build.sh writes its CH-loadable kernel to $A_OUT/kernel on BOTH
# arches (x86_64 copies vmlinuz verbatim — it is already a bzImage; aarch64
# unwraps the UKI). increment-d looked for $A_OUT/Image, which only ever existed
# under an older aarch64-only layout.
SRC_KERNEL="$A_OUT/kernel"
[ -f "$SRC_KERNEL" ] || { echo "!!! missing $SRC_KERNEL — run increment-a/build.sh first" >&2; exit 1; }
echo "=== [build] reusing increment-a kernel (no rebuild)"
file -b "$SRC_KERNEL"

mkdir -p "$OUT"

# Stage our OWN copy at 0644 rather than chmod'ing increment-a's artifact.
# increment-a's kernel lands 0600 root:root (it is copied straight out of
# /boot, which is 0600 on Ubuntu). increment-e drops CH to the unprivileged
# `spikevmm`, which then cannot open it:
#   Error booting VM: VmBoot(KernelFile(Os { code: 13, kind: PermissionDenied }))
# That is plain DAC, NOT landlock — CH auto-derives a landlock rule for
# --kernel, so the ruleset was never the blocker. Copying keeps increment-a's
# artifacts untouched as evidence and makes increment-e self-contained.
install -m 0644 "$SRC_KERNEL" "$OUT/kernel"
echo "=== [build] staged kernel for the unprivileged VMM: $(stat -c '%A %U:%G %s' "$OUT/kernel")"
find "$HERE/probe/src" -name '*.rs' -exec touch {} +

echo
echo "=== [build] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
file "$BIN_DIR/guest-init-fs" "$BIN_DIR/host-collector"

echo
echo "=== [build] staging rootfs tree"
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
  [ -f "$src" ] || { echo "!!! missing module $src" >&2; exit 1; }
  zstd -d -f -q "$src" -o "$STAGE/modules/$name.ko"
  echo "    staged $name.ko ($(stat -c %s "$STAGE/modules/$name.ko") bytes)"
done

# The kernel opens /dev/console as fd 0/1/2 for init BEFORE devtmpfs is
# mounted, so the node must exist statically in the image.
mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"
chmod 666 "$STAGE/dev/null"

echo
echo "=== [build] mkfs.ext4 -d (no loop mount, no root needed for population)"
rm -f "$OUT/rootfs.ext4"
truncate -s 96M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-e -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4"
debugfs -R "ls -l /modules" "$OUT/rootfs.ext4" 2>/dev/null || true
echo "=== [build] DONE"
