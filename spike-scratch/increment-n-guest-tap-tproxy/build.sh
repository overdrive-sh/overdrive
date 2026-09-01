#!/usr/bin/env bash
# PROBE increment-n — build the static host listener + static guest /init, and
# the ext4 virtio-blk rootfs (single /init, no shell/busybox). Run as root on
# the metal box. Artifacts land on a NATIVE fs (not the rsync mount) because
# mkfs -d + mknod need it.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-n
STAGE="$OUT/rootfs"
mkdir -p "$OUT"

echo "=== [build] gcc -static host-listener + guest-init"
gcc -static -O2 -Wall "$HERE/listener.c"   -o "$OUT/host-listener"
gcc -static -O2 -Wall "$HERE/guest-init.c" -o "$OUT/guest-init"
file "$OUT/host-listener" "$OUT/guest-init"
# Prove they are truly static (no interpreter).
ldd "$OUT/guest-init" 2>&1 || true

echo
echo "=== [build] CH-loadable kernel = the box's own bzImage"
KVER="${KVER:-$(uname -r)}"
KSRC="/boot/vmlinuz-${KVER}"
[ -f "$KSRC" ] || { echo "no kernel at $KSRC" >&2; exit 1; }
cp "$KSRC" "$OUT/kernel"
file "$OUT/kernel"

echo
echo "=== [build] staging rootfs tree"
rm -rf "$STAGE"
mkdir -p "$STAGE"/{proc,sys,dev,tmp}
install -m 0755 "$OUT/guest-init" "$STAGE/init"
# /dev/console is opened as init's fd0/1/2 BEFORE devtmpfs mounts, so it must
# exist statically. /dev/null for good measure.
mknod "$STAGE/dev/console" c 5 1 && chmod 600 "$STAGE/dev/console"
mknod "$STAGE/dev/null"    c 1 3 && chmod 666 "$STAGE/dev/null"

echo
echo "=== [build] mkfs.ext4 -d (no loop mount)"
rm -f "$OUT/rootfs.ext4"
truncate -s 64M "$OUT/rootfs.ext4"
mkfs.ext4 -F -L spikeroot -d "$STAGE" "$OUT/rootfs.ext4" >/dev/null
debugfs -R "ls -l /" "$OUT/rootfs.ext4" 2>/dev/null | sed 's/^/    /' || true
echo "=== [build] DONE ($OUT)"
