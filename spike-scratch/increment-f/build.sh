#!/usr/bin/env bash
# PROBE increment-f — the virtio-blk VOLUME counterfactual to increment-e.
#
# I-6 splits storage by role (block rootfs, virtiofs volumes). The rootfs half
# is argued from measurement; the volume half was never measured against the
# block alternative. This builds the same probe over a second virtio-blk device
# so the comparison is a number.
#
# REUSES increment-a's CH-loadable kernel, same as increment-e, so all three
# increments boot the SAME kernel.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-f
STAGE="$OUT/rootfs"
ARCH="$(uname -m)"
TARGET="${ARCH}-unknown-linux-musl"
KVER="${KVER:-$(uname -r)}"
# Volume images live where the volumes themselves would: the XFS reflink volume.
# increment-e showed the numbers track the device, so both increments must
# measure on the same one.
VOLROOT="${VOLROOT:-/srv/vm/p6f}"

_HOME="$(getent passwd "${SUDO_USER:-$(id -un)}" | cut -d: -f6)"
export RUSTUP_HOME="${RUSTUP_HOME:-${_HOME}/.rustup}"
export CARGO_HOME="${CARGO_HOME:-${_HOME}/.cargo}"
export PATH="${CARGO_HOME}/bin:${PATH}"
export CARGO_TARGET_DIR="$HERE/target"

echo "=== [build] arch=$ARCH target=$TARGET kver=$KVER volroot=$VOLROOT"

SRC_KERNEL="$A_OUT/kernel"
[ -f "$SRC_KERNEL" ] || { echo "!!! missing $SRC_KERNEL — run increment-a/build.sh first" >&2; exit 1; }
mkdir -p "$OUT" "$VOLROOT"
# 0644 copy: CH runs as the unprivileged spikevmm and cannot open /boot's 0600.
install -m 0644 "$SRC_KERNEL" "$OUT/kernel"
echo "=== [build] kernel staged: $(stat -c '%A %U:%G %s' "$OUT/kernel")"

find "$HERE/probe/src" -name '*.rs' -exec touch {} +
echo
echo "=== [build] cargo build --release --target $TARGET"
cargo build --release --target "$TARGET" --manifest-path "$HERE/probe/Cargo.toml"
BIN_DIR="$CARGO_TARGET_DIR/$TARGET/release"
file "$BIN_DIR/guest-init-blk" "$BIN_DIR/host-collector"

echo
echo "=== [build] staging rootfs tree"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt,modules}
install -m 0755 "$BIN_DIR/guest-init-blk" "$STAGE/init"

echo "=== [build] kernel modules"
grep -E 'CONFIG_(EXT4_FS|VIRTIO_BLK|VSOCKETS|VIRTIO_VSOCKETS)=' "/boot/config-$KVER" || true
# ONLY the three vsock modules. increment-e additionally had to stage
# virtiofs.ko because CONFIG_VIRTIO_FS=m; ext4 and virtio_blk are both =y, so
# the block volume path needs no module staging at all. That difference is
# evidence, not incidental.
for spec in \
  "net/vmw_vsock/vsock" \
  "net/vmw_vsock/vmw_vsock_virtio_transport_common" \
  "net/vmw_vsock/vmw_vsock_virtio_transport"
do
  name="$(basename "$spec")"
  src="/lib/modules/$KVER/kernel/$spec.ko.zst"
  [ -f "$src" ] || { echo "!!! missing module $src" >&2; exit 1; }
  zstd -d -f -q "$src" -o "$STAGE/modules/$name.ko"
  echo "    staged $name.ko ($(stat -c %s "$STAGE/modules/$name.ko") bytes)"
done

mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"
chmod 666 "$STAGE/dev/null"

echo
echo "=== [build] mkfs.ext4 -d rootfs"
rm -f "$OUT/rootfs.ext4"
truncate -s 96M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-f -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4"

##########################################################################
# The VOLUME images. Seeded with exactly the same host-side content
# increment-e's virtiofs shares carry, so the round-trip assertions are
# byte-for-byte the same test.
echo
echo "=== [build] volume images (the increment-e shares, as block devices)"
VSTAGE_RW="$OUT/volstage-rw"; VSTAGE_RO="$OUT/volstage-ro"
rm -rf "$VSTAGE_RW" "$VSTAGE_RO"; mkdir -p "$VSTAGE_RW" "$VSTAGE_RO"
printf 'HOST-WROTE-THIS-9876543210-zyxwvutsrq\n' >"$VSTAGE_RW/from-host.txt"
printf 'PREEXISTING-HOST-CONTENT-DO-NOT-CHANGE\n' >"$VSTAGE_RO/preexisting-host-file.txt"

# Sized to hold the 256 MiB payload plus the 1000 small files with room to
# spare; ENOSPC would masquerade as a virtio-blk failure.
rm -f "$VOLROOT/volrw.ext4" "$VOLROOT/volro.ext4"
truncate -s 1G "$VOLROOT/volrw.ext4"
truncate -s 64M "$VOLROOT/volro.ext4"
mkfs.ext4 -F -L volrw -d "$VSTAGE_RW" "$VOLROOT/volrw.ext4" >/dev/null
mkfs.ext4 -F -L volro -d "$VSTAGE_RO" "$VOLROOT/volro.ext4" >/dev/null
# Pristine masters. run.sh reflink-clones these per launch, which is how a
# block volume would actually be provisioned (P4: ~260x cheaper than a copy).
chmod 0644 "$VOLROOT/volrw.ext4" "$VOLROOT/volro.ext4"
ls -la "$VOLROOT"/vol*.ext4
echo "    volume fs: $(findmnt -no FSTYPE --target "$VOLROOT") on $(findmnt -no SOURCE --target "$VOLROOT")"
echo "=== [build] DONE"
