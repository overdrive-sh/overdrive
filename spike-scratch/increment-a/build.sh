#!/usr/bin/env bash
# PROBE increment-a — build the static guest init, the host listener, and the
# ext4 virtio-blk rootfs. Run as root inside the overdrive Lima VM.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The repo is virtiofs-mounted into the VM and virtiofs refuses mknod(2), so the
# rootfs staging tree and image live on a native VM filesystem. Source stays on
# the mount; artifacts do not.
OUT=/var/tmp/spike-increment-a
STAGE="$OUT/rootfs"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"

# Derive from the invoking user rather than hardcoding the Lima account —
# on the metal box the build user is `ubuntu`.
_HOME="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
export RUSTUP_HOME="${RUSTUP_HOME:-${_HOME}/.rustup}"
export CARGO_HOME="${CARGO_HOME:-${_HOME}/.cargo}"
export PATH="${CARGO_HOME}/bin:${PATH}"
# Keep the spike OUT of the shared workspace target dir.
export CARGO_TARGET_DIR="$HERE/target"

mkdir -p "$OUT"

# The repo is virtiofs-mounted from macOS; host/guest mtime skew makes cargo
# consider edited sources up to date. Touch them guest-side so a probe edit
# cannot silently run the previous binary.
find "$HERE/probe/src" -name '*.rs' -exec touch {} +

echo "=== [build] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"

BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
echo
echo "=== [build] resulting binaries"
file "$BIN_DIR/guest-init" "$BIN_DIR/host-listener"
ldd "$BIN_DIR/guest-init" 2>&1 || true

echo
echo "=== [build] preparing a CH-loadable kernel for ${ARCH}"
# Cloud Hypervisor's accepted kernel format differs by arch (linux_loader):
#   x86_64  -> bzImage, or a PVH-enabled vmlinux ELF. Ubuntu's /boot/vmlinuz-*
#              IS a bzImage, so it loads DIRECTLY — no unwrapping.
#   aarch64 -> raw PE `Image`. Ubuntu's arm64 vmlinuz is a UKI whose .linux
#              section holds a nested EFI-zboot PE holding a zstd-compressed
#              Image, so BOTH layers must be peeled. Handing CH the UKI fails
#              with `VmBoot(UefiLoad(UefiTooBig))` — a misleading error that
#              says nothing about the actual format problem.
KVER="${KVER:-$(uname -r)}"
KERNEL_SRC="${KERNEL_SRC:-/boot/vmlinuz-${KVER}}"
[ -f "$KERNEL_SRC" ] || { echo "no kernel at $KERNEL_SRC" >&2; exit 1; }

case "$ARCH" in
  x86_64)
    cp "$KERNEL_SRC" "$OUT/kernel"
    echo "--- x86_64: using vmlinuz as-is"
    file "$OUT/kernel"
    ;;
  aarch64)
    python3 "$HERE/inspect_kernel.py" "$KERNEL_SRC"
    zstd -d -f "$OUT/payload.bin" -o "$OUT/kernel"
    echo "--- aarch64: unwrapped UKI -> EFI-zboot -> zstd -> raw Image"
    file "$OUT/kernel"
    echo -n "--- arm64 Image magic @0x38: "; xxd -s 0x38 -l 4 "$OUT/kernel"
    ;;
  *) echo "unsupported arch: $ARCH" >&2; exit 1 ;;
esac

echo
echo "=== [build] staging rootfs tree"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,dev,tmp,modules}
install -m 0755 "$BIN_DIR/guest-init" "$STAGE/init"

# Ubuntu builds CONFIG_VSOCKETS/CONFIG_VIRTIO_VSOCKETS as MODULES (=m), so a
# module-free rootfs gets EAFNOSUPPORT on socket(AF_VSOCK). Ship the three
# modules, zstd-decompressed (finit_module takes uncompressed ELF).
# KVER already derived above from `uname -r`.
for m in vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
  src="/lib/modules/$KVER/kernel/net/vmw_vsock/$m.ko.zst"
  zstd -d -f -q "$src" -o "$STAGE/modules/$m.ko"
  echo "    staged module $m.ko ($(stat -c %s "$STAGE/modules/$m.ko") bytes) from $src"
done

# The kernel opens /dev/console as fd 0/1/2 for init BEFORE devtmpfs is
# mounted, so the node must exist statically in the image.
mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"
chmod 666 "$STAGE/dev/null"

echo
echo "=== [build] mkfs.ext4 (populate via -d, no loop mount needed)"
rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot -d "$STAGE" "$OUT/rootfs.ext4"

echo
echo "=== [build] rootfs image"
file "$OUT/rootfs.ext4"
ls -la "$OUT/rootfs.ext4"
echo "=== [build] image contents (debugfs)"
debugfs -R "ls -l /" "$OUT/rootfs.ext4" 2>/dev/null || true
echo "=== [build] DONE"
