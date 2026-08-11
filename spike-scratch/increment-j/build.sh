#!/usr/bin/env bash
# PROBE increment-j — build the vsock-across-snapshot guest, its rootfs, and the
# host-side listener.
#
# Reuses increment-a's CH-loadable kernel (same as e/f/g/h), so this is measured
# against the SAME booting VM P1/P2 validated on this box.
#
# The three vsock modules are the load-bearing part of the staging. Ubuntu ships
# CONFIG_VSOCKETS=m, so a rootfs without them gives EAFNOSUPPORT on
# socket(AF_VSOCK) — which reads as "vsock is unsupported" and is really a
# missing file. Two earlier probes in this spike recorded exactly that shape of
# wrong negative, so the build FAILS LOUDLY if any module is missing rather than
# staging a rootfs that will quietly under-report.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-j
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
file "$BIN_DIR/guest-init-vsock-snap"
file "$BIN_DIR/vsock-listener"

echo "=== [build] staging rootfs"
rm -rf "$STAGE"; mkdir -p "$STAGE"/{proc,sys,dev,tmp,mnt,modules}
install -m 0755 "$BIN_DIR/guest-init-vsock-snap" "$STAGE/init"

KVER="${KVER:-$(uname -r)}"
for spec in \
  "net/vmw_vsock/vsock" \
  "net/vmw_vsock/vmw_vsock_virtio_transport_common" \
  "net/vmw_vsock/vmw_vsock_virtio_transport"
do
  name="$(basename "$spec")"
  src="/lib/modules/$KVER/kernel/$spec.ko.zst"
  [ -f "$src" ] || { echo "!!! MISSING MODULE $src -- refusing to stage a rootfs that will report a false negative" >&2; exit 1; }
  zstd -d -f -q "$src" -o "$STAGE/modules/$name.ko"
  echo "    staged $name.ko ($(stat -c %s "$STAGE/modules/$name.ko") bytes)"
done

mknod "$STAGE/dev/console" c 5 1
mknod "$STAGE/dev/null" c 1 3
chmod 600 "$STAGE/dev/console"; chmod 666 "$STAGE/dev/null"

rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot-j -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
ls -la "$OUT/rootfs.ext4"
install -m 0755 "$BIN_DIR/vsock-listener" "$OUT/vsock-listener"
echo "=== [build] DONE"
