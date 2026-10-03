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

echo "=== /init (Spike E, phase-dispatched by igkm_phase= on the kernel cmdline) ==="
cat > "$ROOT/init" <<'INIT'
#!/bin/sh
export PATH=/bin:/sbin
mount -t proc     proc /proc
mount -t sysfs    sys  /sys
mount -t devtmpfs dev  /dev 2>/dev/null
ip link set lo up 2>/dev/null
ip link set eth0 up
ip addr add 192.168.204.2/24 dev eth0
PHASE=regression
for tok in $(cat /proc/cmdline); do
  case "$tok" in igkm_phase=*) PHASE="${tok#igkm_phase=}";; esac
done
echo "IGKME-INIT: booted $(uname -r) phase=$PHASE; eth0 $(ip -o -4 addr show eth0 2>/dev/null)"
for m in tls vsock vmw_vsock_virtio_transport_common vmw_vsock_virtio_transport; do
  insmod /mod/$m.ko && echo "IGKME-INIT: insmod $m OK" || echo "IGKME-INIT: insmod $m FAIL rc=$?"
done
# mesh        = client VIPs the app dials (resolve-over-vsock + dst rewrite).
# mesh_listen = local LISTEN ports that take inbound server-side mTLS (Spike E).
insmod /mod/igkmd_mtls.ko \
  mesh=10.80.0.1:9443,10.80.0.2:9443,10.80.0.9:9443 \
  mesh_listen=8443,8444 agent_port=7100 agent_cid=2 \
  && echo "IGKME-INIT: insmod igkmd_mtls OK" || echo "IGKME-INIT: insmod igkmd_mtls FAIL rc=$?"

ktls_dump() {
  echo "IGKME-INIT: in-guest kTLS proof -- /proc/net/tls_stat (nonzero TxSw/RxSw = kernel kTLS active):"
  cat /proc/net/tls_stat 2>/dev/null || echo "  (no /proc/net/tls_stat)"
}

case "$PHASE" in
  regression)
    echo "IGKME-INIT: ===== CASE passthrough (non-mesh 192.168.204.1:5001, plaintext positive control) ====="
    /app/igkmd_app passthrough 192.168.204.1 5001 pt
    echo "IGKME-INIT: ===== CASE lbfresh (VIP 10.80.0.1:9443 -> svc-a; re-resolve after health toggle) ====="
    /app/igkmd_app lbfresh 10.80.0.1 9443 svca
    echo "IGKME-INIT: ===== CASE deny (VIP 10.80.0.9:9443 -> svc-denied; resolve DENY -> connect() EACCES) ====="
    /app/igkmd_app deny 10.80.0.9 9443 dn
    ktls_dump
    ;;
  accept)
    echo "IGKME-INIT: ===== PART 1 inbound accept() server on :8443 (blocking); runner dials 2 inbound callers ====="
    /app/igkmd_app server 8443 2
    ktls_dump
    ;;
  nbconnect)
    echo "IGKME-INIT: ===== PART 2 non-blocking connect (ok): VIP 10.80.0.1:9443 -> svc-a ====="
    /app/igkmd_app nbclient 10.80.0.1 9443 ok
    echo "IGKME-INIT: ===== PART 2 non-blocking connect (handshake-deny -> SO_ERROR): VIP 10.80.0.2:9443 -> svc-hs-deny ====="
    /app/igkmd_app nbclient 10.80.0.2 9443 hsdeny
    echo "IGKME-INIT: ===== PART 2 non-blocking connect (resolve-deny -> connect EACCES): VIP 10.80.0.9:9443 ====="
    /app/igkmd_app nbclient 10.80.0.9 9443 rdeny
    ktls_dump
    ;;
  nbaccept)
    echo "IGKME-INIT: ===== PART 2 non-blocking accept + edge-triggered epoll server on :8444 (runner floods it) ====="
    /app/igkmd_app nbserver 8444 40000
    ktls_dump
    ;;
  *)
    echo "IGKME-INIT: unknown phase '$PHASE'"
    ;;
esac
echo "IGKME-INIT: VERDICT phase=$PHASE complete"
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
