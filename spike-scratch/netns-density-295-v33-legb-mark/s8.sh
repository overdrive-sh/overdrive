#!/bin/bash
# Runs on the metal host as root, UNDER THE CANONICAL LEASE (launch.py). Private dir /tmp/v33-work only.
#   phase A: S1..S7 + Q3 on the pinned-kernel family, native x86_64 (run.sh, ip/nft via CLI)
#   phase B: S8-equivalent with a REAL Cloud Hypervisor microVM behind a leg-C stand-in
set -u
W=/tmp/v33-work
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
EV=$W/evidence/run-$STAMP
mkdir -p "$EV"
BINDIR=$W/out/target/x86_64-unknown-linux-gnu/release
export BIN=$BINDIR RULES=cli
KERNEL=${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}
CHB=/tmp/v28-work/out/cloud-hypervisor
CID=4331
VMPID=
cleanup() {
  [ -n "$VMPID" ] && kill -9 -- -"$VMPID" 2>/dev/null
  ip netns del v33ns 2>/dev/null
  for p in $(pgrep -x v33) $(pgrep -x tcpdump); do kill "$p" 2>/dev/null; done
  return 0
}
trap cleanup EXIT

echo "=== substrate (fail closed)"
echo "uname_r=$(uname -r) machine=$(uname -m) virt=$(systemd-detect-virt)"
[ "$(uname -m)" = x86_64 ] || { echo "FAIL not x86_64"; exit 3; }
[ "$(systemd-detect-virt)" = none ] || { echo "FAIL virtualised"; exit 3; }
[ -c /dev/kvm ] || { echo "FAIL no /dev/kvm"; exit 3; }
grep -m1 'model name' /proc/cpuinfo
ls -l /dev/kvm /dev/vhost-vsock
echo "ip rules / routes in the ROOT netns are untouched by this run; all rules live in netns v33ns"
sha256sum "$BINDIR/v33" "$CHB" "$KERNEL"
"$CHB" --version 2>&1 | head -2
ip netns list

echo; echo "################ PHASE A: S1..S7, Q3 on native metal ($(uname -r))"
for p in S1 S2 S3 S4 S5 S6 S6x S7 Q3; do
  mkdir -p "$EV/$p"
  OUT="$EV/$p" timeout 120 bash "$W/run.sh" "$p" > "$EV/$p.txt" 2>&1
  echo "phase $p rc=$? -> $EV/$p.txt"
done

echo; echo "################ PHASE B: S8-equivalent with a real microVM"
IR=$W/out/initramfs-v33
rm -rf "$IR"; cp -a /tmp/v28-work/out/initramfs-root "$IR"
rm -f "$IR/legs"
cp "$BINDIR/v33" "$IR/v33"
sed -i 's#^/legs agent > /tmp/agent.out 2>&1 &#/v33 vm-guest --vport 5000 > /dev/console 2>\&1 \&#' "$IR/init"
grep -n "v33\|GUEST_READY" "$IR/init"
( cd "$IR" && find . | cpio -o -H newc --quiet | gzip -1 > "$W/out/initramfs-v33.cpio.gz" )
SERIAL=$EV/serial.log
setsid "$CHB" --kernel "$KERNEL" --initramfs "$W/out/initramfs-v33.cpio.gz" --cmdline "console=ttyS0 panic=-1 loglevel=4" \
  --cpus boot=2 --memory size=512M --serial "file=$SERIAL" --console off \
  --vsock "cid=$CID,backend=vhost-kernel" --log-file "$EV/ch.log" -v > "$EV/ch.stdout" 2>&1 &
VMPID=$!
echo "CH pid=$VMPID cid=$CID"
ok=0
for i in $(seq 300); do
  if grep -q GUEST_VSOCK_READY "$SERIAL" 2>/dev/null; then ok=1; break; fi
  kill -0 "$VMPID" 2>/dev/null || break
  sleep 0.2
done
echo "guest ready=$ok"
grep -E "GUEST_BOOT|GUEST_VSOCK_READY|INSMOD_FAIL" "$SERIAL" 2>/dev/null
if [ $ok = 1 ]; then
  mkdir -p "$EV/S8"
  OUT="$EV/S8" VM_CID=$CID timeout 120 bash "$W/run.sh" S8 > "$EV/S8.txt" 2>&1
  echo "phase S8 rc=$? -> $EV/S8.txt"
  echo "--- guest serial (served lines):"
  grep -E "GUEST_SERVED|GUEST_VSOCK_READY|GUEST_BOOT" "$SERIAL"
fi
kill -9 -- -"$VMPID" 2>/dev/null; VMPID=
sleep 0.3
echo "--- leftovers:"; ip netns list; pgrep -af 'cloud-hypervisor|v33' | grep -v pgrep
echo "EVIDENCE_DIR=$EV"
