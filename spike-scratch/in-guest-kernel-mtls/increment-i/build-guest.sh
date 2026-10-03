#!/usr/bin/env bash
# Spike D (GH #303, increment-i): build a minimal Linux initramfs (cpio.gz) for
# the STOCK guest kernel (== host kernel, Firecracker boots it directly). Static
# busybox + the stock tls/vsock .ko (decompressed from the host's own .ko.zst,
# same vermagic) + the increment-i module + the guest app. No kernel build, no
# package install. Runs as the login user (--no-sudo).
#   build-guest.sh mesh  -> out/initramfs-mesh.cpio.gz
set -euo pipefail
VARIANT="${1:-mesh}"
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KREL="$(uname -r)"
MODSRC="/lib/modules/$KREL/kernel"
OUT="$HOME/igkm-spike-a/igkmd/out"
mkdir -p "$OUT"
ROOT="$OUT/initramfs-$VARIANT.root"
BB="$(command -v busybox)"

[[ "$VARIANT" == mesh ]] || { echo "FATAL: increment-i only builds the 'mesh' variant"; exit 2; }

rm -rf "$ROOT"; mkdir -p "$ROOT"/{bin,sbin,proc,sys,dev,mod,app,etc}

echo "=== static busybox ==="
file "$BB" | grep -q 'statically linked' || { echo "FATAL: busybox not static"; exit 2; }
install -m0755 "$BB" "$ROOT/bin/busybox"
for ap in sh mount umount insmod rmmod lsmod modprobe dmesg poweroff cat grep sleep \
          echo ls mkdir mknod uname ip ifconfig route hexdump head tail cut tr wc od sync; do
  ln -sf busybox "$ROOT/bin/$ap"
done

echo "=== stock tls + vsock modules (decompress host .ko.zst, same vermagic) ==="
stage_mod() { # relpath-under-kernel
  local z="$MODSRC/$1" b
  b="$(basename "$1" .zst)"
  [[ -f "$z" ]] || { echo "FATAL: missing $z"; exit 2; }
  zstd -q -f -d "$z" -o "$ROOT/mod/$b"
  echo "  staged $b ($(stat -c%s "$ROOT/mod/$b") B)"
}
stage_mod net/tls/tls.ko.zst
stage_mod net/vmw_vsock/vsock.ko.zst
stage_mod net/vmw_vsock/vmw_vsock_virtio_transport_common.ko.zst
stage_mod net/vmw_vsock/vmw_vsock_virtio_transport.ko.zst

echo "=== increment-i module + app ==="
install -m0644 "$OUT/module/igkmd_mtls.ko" "$ROOT/mod/igkmd_mtls.ko"
install -m0755 "$OUT/bin/igkmd_app" "$ROOT/app/igkmd_app"

echo "=== /init (mesh, Spike D: dial VIPs, module resolves to a backend over vsock) ==="
cat > "$ROOT/init" <<'INIT'
#!/bin/sh
export PATH=/bin:/sbin
mount -t proc     proc /proc
mount -t sysfs    sys  /sys
mount -t devtmpfs dev  /dev 2>/dev/null
ip link set lo up 2>/dev/null
echo "IGKMD-INIT: booted $(uname -r); links:"; ip -o link show
ip link set eth0 up
ip addr add 192.168.204.2/24 dev eth0
echo "IGKMD-INIT: eth0 $(ip -o -4 addr show eth0 2>/dev/null)"
for m in tls vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
  insmod /mod/$m.ko && echo "IGKMD-INIT: insmod $m OK" || echo "IGKMD-INIT: insmod $m FAIL rc=$?"
done
echo "IGKMD-INIT: /dev/vsock=$(ls -l /dev/vsock 2>&1)"
# mesh = the stable service VIPs the guest dials (NOT backend addrs). A connect
# to one of these triggers the module's resolve-over-vsock + destination rewrite.
insmod /mod/igkmd_mtls.ko mesh=10.80.0.1:9443,10.80.0.9:9443 agent_port=7100 agent_cid=2 \
  && echo "IGKMD-INIT: insmod igkmd_mtls OK" || echo "IGKMD-INIT: insmod igkmd_mtls FAIL rc=$?"
echo "IGKMD-INIT: ===== CASE passthrough (non-mesh 192.168.204.1:5001, plaintext positive control) ====="
/app/igkmd_app passthrough 192.168.204.1 5001 pt
echo "IGKMD-INIT: ===== CASE lbfresh (VIP 10.80.0.1:9443 -> svc-a; connect-1 lands on first-healthy; host toggles health; connect-2 re-resolves) ====="
/app/igkmd_app lbfresh 10.80.0.1 9443 svca
echo "IGKMD-INIT: ===== CASE deny (VIP 10.80.0.9:9443 -> svc-denied; resolve DENY -> connect() EACCES, no TCP) ====="
/app/igkmd_app deny 10.80.0.9 9443 dn
echo "IGKMD-INIT: in-guest kTLS proof -- /proc/net/tls_stat (nonzero TxSw/RxSw = kernel kTLS active):"
cat /proc/net/tls_stat 2>/dev/null || echo "  (no /proc/net/tls_stat)"
echo "IGKMD-INIT: ss -K (tls ULP on the workload sockets):"
ss -K 2>/dev/null | grep -a -i tls || echo "  (ss has no -K / no tls sockets listed)"
echo "IGKMD-INIT: VERDICT mesh-complete"
sync
reboot -f
INIT
chmod 0755 "$ROOT/init"

echo "=== pack cpio.gz ==="
( cd "$ROOT" && find . -print0 | cpio --null -o --format=newc 2>/dev/null ) | gzip -9 > "$OUT/initramfs-$VARIANT.cpio.gz"
ls -l "$OUT/initramfs-$VARIANT.cpio.gz"
sha256sum "$OUT/initramfs-$VARIANT.cpio.gz"
echo "=== contents ==="
( cd "$ROOT" && find . | sort | sed 's/^/  /' )
