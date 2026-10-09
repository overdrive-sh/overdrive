#!/usr/bin/env bash
# V-10 host orchestrator (THROWAWAY SPIKE, netns-density-295). Runs as root on
# the native x86_64 metal host under the canonical flock lease. Builds the
# apparatus, boots one guest per scenario with the fork CH + kernel vhost-vsock
# backend, and records what the host kernel attributes a forged-src_cid packet
# to.
#
# Usage (held under: sudo flock -w N /run/lock/overdrive-metal-shared.lock):
#   bash driver.sh <base_dir>
set -u
BASE="${1:?base dir}"
EV="$BASE/evidence"
mkdir -p "$EV"
KVER="$(uname -r)"
CID=3271                   # attacker VM CID (this spike owns 3270..3279)
SPORT=1234; QPORT=1235
RESULTS="$EV/results.json"

log(){ echo "[driver $(date +%H:%M:%S)] $*"; }
jadd(){ printf '%s\n' "$1" >> "$EV/results.jsonl"; }

# ---- what we loaded, so cleanup only removes our own state ----
LOADED_VHOST=0
LOADED_VSOCKMON=0
VSOCKMON_LINK=""
CH_PID=""

cleanup(){
  set +e
  log "cleanup"
  [ -n "$CH_PID" ] && kill -9 "$CH_PID" 2>/dev/null
  pkill -9 -f "cloud-hypervisor.*cid=$CID" 2>/dev/null
  pkill -9 -f "$BASE/capture.py" 2>/dev/null
  pkill -9 -f "$BASE/host_listener" 2>/dev/null
  [ -n "$VSOCKMON_LINK" ] && ip link del "$VSOCKMON_LINK" 2>/dev/null
  if [ "$LOADED_VSOCKMON" = "1" ]; then rmmod vsockmon 2>/dev/null && log "rmmod vsockmon (we loaded it)"; fi
  if [ "$LOADED_VHOST" = "1" ]; then rmmod vhost_vsock 2>/dev/null && log "rmmod vhost_vsock (we loaded it)"; fi
  ( echo "=== modules after cleanup ==="; lsmod | grep -E 'vsock|vhost' || true ) > "$EV/inventory-after.txt" 2>&1
}
trap cleanup EXIT

# -------------------------------------------------- fail-closed substrate gate
log "substrate preflight"
{
  echo "uname_r=$KVER"
  echo "arch=$(uname -m)"
  echo "virt=$(systemd-detect-virt || true)"
} > "$EV/substrate.txt"
[ "$(uname -m)" = "x86_64" ] || { echo "FATAL arch not x86_64"; exit 1; }
[ -c /dev/kvm ] || { echo "FATAL /dev/kvm missing"; exit 1; }
python3 - <<'PY' || { echo "FATAL kvm probe"; exit 1; }
import fcntl,os
fd=os.open("/dev/kvm",os.O_RDWR|os.O_CLOEXEC)
v=fcntl.ioctl(fd,0xAE00,0)
assert v==12, v
vm=fcntl.ioctl(fd,0xAE01,0); os.close(vm); os.close(fd)
PY
V="$(systemd-detect-virt || true)"
[ "$V" = "none" ] || { echo "FATAL virt=$V (must be none)"; exit 1; }
log "substrate OK x86_64 /dev/kvm virt=none kver=$KVER"

# -------------------------------------------------- locate prebuilt fork CH
# Under `sudo`, $HOME is /root; the prebuilt CH lives in the login user's home.
LOGIN_HOME="$(eval echo ~"${SUDO_USER:-ubuntu}")"
CH=""
for c in \
  "$LOGIN_HOME/overdrive/spike-scratch/netns-density-295-guest-vsock-capture/out/ch-target/release/cloud-hypervisor" \
  "$LOGIN_HOME/overdrive/spike-scratch/netns-density-295-ch-vhost-vsock/out/ch-target/release/cloud-hypervisor" \
  "$LOGIN_HOME/overdrive/spike-scratch/netns-density-295-v24/out/ch-target/release/cloud-hypervisor" ; do
  [ -x "$c" ] && { CH="$c"; break; }
done
[ -n "$CH" ] || { echo "FATAL no prebuilt fork CH found"; exit 1; }
CH_SHA="$(sha256sum "$CH" | cut -d' ' -f1)"
log "fork CH=$CH sha256=$CH_SHA ver=$("$CH" --version 2>&1 | head -1)"

GK="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
[ -r "$GK" ] || { echo "FATAL guest kernel $GK unreadable"; exit 1; }
GK_SHA="$(sha256sum "$GK" | cut -d' ' -f1)"

# -------------------------------------------------- build apparatus
log "build forge.ko"
make -C "$BASE/forge" KDIR="/lib/modules/$KVER/build" >"$EV/build-forge.log" 2>&1 || { cat "$EV/build-forge.log"; echo FATAL build forge; exit 1; }
cp "$BASE/forge/vsock_forge.ko" "$BASE/forge.ko"
modinfo "$BASE/forge.ko" | grep -E 'vermagic|name|parm' > "$EV/forge-modinfo.txt" 2>&1

log "build guest_client (static)"
if ! gcc -O2 -static -o "$BASE/client" "$BASE/guest_client.c" >"$EV/build-client.log" 2>&1; then
  log "static build failed; retry dynamic"
  gcc -O2 -o "$BASE/client" "$BASE/guest_client.c" >>"$EV/build-client.log" 2>&1 || { cat "$EV/build-client.log"; echo FATAL build client; exit 1; }
fi
file "$BASE/client" > "$EV/client-file.txt"

log "build host_listener"
gcc -O2 -o "$BASE/host_listener" "$BASE/host_listener.c" >"$EV/build-listener.log" 2>&1 || { cat "$EV/build-listener.log"; echo FATAL build listener; exit 1; }

# -------------------------------------------------- assemble guest initramfs
log "assemble initramfs"
ROOT="$BASE/initramfs-root"; rm -rf "$ROOT"; mkdir -p "$ROOT"/{proc,sys,dev,bin,mods}
cp /usr/bin/busybox "$ROOT/bin/busybox"
for a in sh mount insmod rmmod cat echo uname sleep grep sync poweroff dmesg ls cut lsmod; do
  ln -sf busybox "$ROOT/bin/$a"
done
cp "$BASE/client" "$ROOT/client"; chmod 755 "$ROOT/client"
cp "$BASE/forge.ko" "$ROOT/forge.ko"
cp "$BASE/guest-init.sh" "$ROOT/init"; chmod 755 "$ROOT/init"
: > "$ROOT/mods/order"
modprobe --show-depends vmw_vsock_virtio_transport 2>/dev/null | while read -r verb path _; do
  [ "$verb" = "insmod" ] || continue
  bn="$(basename "$path")"; name="${bn%.zst}"
  if [ "${path##*.}" = "zst" ]; then zstd -dc "$path" > "$ROOT/mods/$name"; else cp "$path" "$ROOT/mods/$name"; fi
  echo "$name" >> "$ROOT/mods/order"
done
cp "$ROOT/mods/order" "$EV/mods-order.txt"
( cd "$ROOT" && find . | cpio -o -H newc --quiet | gzip -1 ) > "$BASE/initramfs.cpio.gz"
INITRD="$BASE/initramfs.cpio.gz"
INITRD_SHA="$(sha256sum "$INITRD" | cut -d' ' -f1)"

# artifact pins
cat > "$EV/artifact-pins.json" <<JSON
{"uname_r":"$KVER","ch":"$CH","ch_sha256":"$CH_SHA","guest_kernel":"$GK","guest_kernel_sha256":"$GK_SHA","initramfs_sha256":"$INITRD_SHA","forge_sha256":"$(sha256sum "$BASE/forge.ko" | cut -d' ' -f1)","client_sha256":"$(sha256sum "$BASE/client" | cut -d' ' -f1)"}
JSON

# -------------------------------------------------- host vsock prerequisites
( echo "=== modules before ==="; lsmod | grep -E 'vsock|vhost' || echo none ) > "$EV/inventory-before.txt"
if ! lsmod | grep -q '^vhost_vsock'; then modprobe vhost_vsock && LOADED_VHOST=1 && log "modprobe vhost_vsock (we loaded it)"; fi
if ! lsmod | grep -q '^vsockmon'; then modprobe vsockmon && LOADED_VSOCKMON=1 && log "modprobe vsockmon (we loaded it)"; fi
VSOCKMON_LINK="v10-vsockmon0"
ip link del "$VSOCKMON_LINK" 2>/dev/null
ip link add "$VSOCKMON_LINK" type vsockmon && ip link set "$VSOCKMON_LINK" up || { echo "FATAL vsockmon link"; exit 1; }
log "vsockmon link $VSOCKMON_LINK up"

# -------------------------------------------------- one scenario
run_scenario(){
  local name="$1" mode="$2" forge="$3"
  local dir="$EV/$name"; mkdir -p "$dir"
  local serial="$dir/serial.log" ll="$dir/listener.log" cap="$dir/capture.log" chlog="$dir/ch.log"
  log "=== scenario $name (mode=$mode forge=$forge cid=$CID) ==="

  python3 "$BASE/capture.py" "$VSOCKMON_LINK" 75 "$cap" >"$dir/capture.stdout" 2>&1 &
  local cpid=$!
  "$BASE/host_listener" "$SPORT" "$QPORT" 70 "$ll" >"$dir/listener.stdout" 2>&1 &
  local lpid=$!
  sleep 2

  local cmdline="console=ttyS0 panic=-1 loglevel=7 v10_mode=$mode v10_guestcid=$CID v10_forge=$forge v10_sport=$SPORT v10_qport=$QPORT"
  "$CH" --kernel "$GK" --initramfs "$INITRD" --cmdline "$cmdline" \
    --cpus boot=1 --memory size=512M --serial "file=$serial" --console off \
    --vsock "cid=$CID,backend=vhost-kernel" --log-file "$chlog" -v >"$dir/ch.stdout" 2>&1 &
  CH_PID=$!

  local waited=0
  while [ $waited -lt 60 ]; do
    grep -q "V10 guest done" "$serial" 2>/dev/null && break
    kill -0 "$CH_PID" 2>/dev/null || break
    sleep 1; waited=$((waited+1))
  done
  sleep 2
  kill -9 "$CH_PID" 2>/dev/null; wait "$CH_PID" 2>/dev/null; CH_PID=""
  wait "$lpid" 2>/dev/null
  kill "$cpid" 2>/dev/null; wait "$cpid" 2>/dev/null

  # extract the observable facts
  local accepts peer_cids forged_wire host_wire
  # NB: `grep -c` exits 1 on zero matches, so `grep -c ... || echo 0` would
  # print "0\n0"; count via `grep | wc -l`, which always emits one integer.
  accepts="$(grep -a 'ACCEPT' "$ll" 2>/dev/null | wc -l | tr -d ' ')"
  peer_cids="$(grep -a 'ACCEPT' "$ll" 2>/dev/null | sed -n 's/.*peer_cid=\([0-9]*\).*/\1/p' | sort -u | tr '\n' ',' )"
  forged_wire="$(grep -a "src_cid=$forge " "$cap" 2>/dev/null | wc -l | tr -d ' ')"
  host_wire="$(grep -aE "src_cid=$CID " "$cap" 2>/dev/null | wc -l | tr -d ' ')"
  log "  listener accepts=$accepts peer_cids=[$peer_cids]"
  log "  wire frames src_cid=$forge (forged): $forged_wire ; src_cid=$CID (real): $host_wire"
  jadd "{\"scenario\":\"$name\",\"mode\":\"$mode\",\"forge\":$forge,\"guest_cid\":$CID,\"listener_accepts\":$accepts,\"listener_peer_cids\":\"$peer_cids\",\"wire_frames_src_forged\":$forged_wire,\"wire_frames_src_real\":$host_wire}"
}

echo "V-10 scenarios on kernel $KVER, CH $CH_SHA" > "$EV/summary.txt"
run_scenario control   plain     0
sleep 3
run_scenario forge3272 plain     3272
sleep 3
run_scenario forge2    plain     2
sleep 3
run_scenario midstream midstream 3272

# -------------------------------------------------- roll up results.json
python3 - "$EV" <<'PY'
import json,sys,os
ev=sys.argv[1]
rows=[]
p=os.path.join(ev,"results.jsonl")
if os.path.exists(p):
    for line in open(p):
        line=line.strip()
        if line: rows.append(json.loads(line))
json.dump({"scenarios":rows}, open(os.path.join(ev,"results.json"),"w"), indent=2)
print(json.dumps(rows, indent=2))
PY
log "done; evidence in $EV"
