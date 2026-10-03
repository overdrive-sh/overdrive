#!/usr/bin/env bash
# Spike E (GH #303, increment-j) BOOT phase, parameterized by PHASE. Runs as
# ROOT (cargo xtask metal run, default sudo). Does NOT build -- build-all.sh must
# have run as the login user (--no-sudo) first. Stages the stock kernel, wires a
# uniquely named tap + tcpdump + host backends (peer, incl. a peer-denied
# backend on :6445) + the host agent/relay (service registry + health dir), and
# boots Firecracker with igkm_phase=<PHASE>. Per phase it drives the host side:
#   regression  (item 6): guest dials out; toggle backend A unhealthy mid-run.
#   accept    (items 1-2): guest is a blocking server :8443; dial an allowed then
#                          a denied inbound caller (host rustls clients).
#   nbconnect   (item 3) : guest dials out non-blocking; no inbound dial.
#   nbaccept    (item 4) : guest is a non-blocking ET server :8444; flood it.
# Scans the tap capture, emits the pcap as base64, cleans up every host object.
set -uo pipefail
PHASE="${1:?usage: run-fc.sh <regression|accept|nbconnect|nbaccept>}"
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRATCH="$HOME/igkm-spike-a"
FC="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
BIN="$SCRATCH/igkmd-host-target/release"
OUT="$SCRATCH/igkmd/out"
KDIR="$SCRATCH/igkmd/kernel"
CERTS="$OUT/certs"
KREL="$(uname -r)"
TAP="igkd$(date -u +%d%H%M%S)"
TAP_IP=192.168.204.1
GUEST_IP=192.168.204.2
GUEST_MAC=52:54:00:1a:05:07
GUEST_CID=3
AGENT_PORT=7100
BOOT_TIMEOUT_S=90
RUN_DIR="$SCRATCH/runs-igkmd/$(date -u +%Y%m%dT%H%M%SZ)-$$"
FC_PID=""; RELAY_PID=""; PEER_PID=""; TCPDUMP_PID=""; TAP_CREATED=0; IC_PIDS=""
BACKEND_A="192.168.204.1_6443"
T0="$(date +%s.%N)"
since() { python3 -c "import time; print(f't+{time.time()-$T0:.3f}s')"; }
section() { printf '\n===== %s =====\n' "$*"; }

stop_pid() { local p="$1" sig="$2"; [[ -n "$p" ]] || return 0
  if kill -0 "$p" 2>/dev/null; then kill -"$sig" "$p"
    for _ in $(seq 1 40); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$p" 2>/dev/null && kill -KILL "$p"; fi; wait "$p" 2>/dev/null; }
cleanup() { set +e; section "cleanup"
  stop_pid "$FC_PID" TERM; FC_PID=""
  for p in $IC_PIDS; do stop_pid "$p" TERM; done; IC_PIDS=""
  stop_pid "$RELAY_PID" TERM; RELAY_PID=""
  stop_pid "$PEER_PID" TERM; PEER_PID=""
  stop_pid "$TCPDUMP_PID" INT; TCPDUMP_PID=""
  [[ "$TAP_CREATED" -eq 1 ]] && ip link show "$TAP" >/dev/null 2>&1 && { echo "del tap $TAP"; ip link del "$TAP"; }
  [[ -d "$RUN_DIR" && "$RUN_DIR" == "$SCRATCH"/runs-igkmd/* ]] && rm -rf -- "$RUN_DIR"
  rmdir "$SCRATCH/runs-igkmd" 2>/dev/null
  section "post-run host state"
  echo "tap $TAP present: $(ip link show "$TAP" >/dev/null 2>&1 && echo yes || echo no)"
  echo "192.168.204.x on host: $(ip -4 -br addr | grep -c '192\.168\.204\.')"
  echo "firecracker procs: [$(pgrep -a firecracker | tr '\n' ';')]"
  echo "peer/relay/inclient procs: [$(pgrep -af "$BIN/(peer|relay|inclient)" | tr '\n' ';')]"
  echo "tcpdump procs: [$(pgrep -a tcpdump | tr '\n' ';')]"; }

section "preconditions (build-all.sh --no-sudo must have produced these)"
for f in "$FC" "$OUT/module/igkmd_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$OUT/bin/igkmd_app" \
         "$BIN/peer" "$BIN/relay" "$BIN/inclient" "$CERTS/guest-server.pem" "$CERTS/peer-client.pem" \
         "$CERTS/peer-client-denied.pem" "$CERTS/peer-denied.pem"; do
  [[ -e "$f" ]] || { echo "FATAL: missing prebuilt artifact: $f"; exit 2; }
done
echo "all prebuilt artifacts present; PHASE=$PHASE"

section "pre-run host state"
ip link show "$TAP" >/dev/null 2>&1 && { echo "FATAL: tap $TAP exists"; exit 2; }
ip -4 addr show | grep -q '192\.168\.204\.' && { echo "FATAL: 192.168.204.0/24 in use"; exit 2; }
ss -Htln | grep -qE ':(5001|6443|6444|6445|7100|8443|8444)\b' && { echo "FATAL: a test port is in use"; exit 2; }
trap cleanup EXIT HUP INT TERM

section "stage stock kernel (guest kernel == host kernel)"
[[ -f "$KDIR/vmlinuz-host" ]] || install -D -m0644 /boot/vmlinuz-"$KREL" "$KDIR/vmlinuz-host"
KERNEL="$KDIR/vmlinuz-host"; INITRD="$OUT/initramfs-mesh.cpio.gz"

section "artifacts"
echo "host uname -r: $KREL"; "$FC" --version | head -1
sha256sum "$KERNEL" "$INITRD" "$OUT/module/igkmd_mtls.ko" "$OUT/bin/igkmd_app" "$BIN/peer" "$BIN/relay" "$BIN/inclient"
echo "module vermagic: $(modinfo -F vermagic "$OUT/module/igkmd_mtls.ko" 2>/dev/null)"

mkdir -p "$RUN_DIR"
VSOCK_BASE="$RUN_DIR/vsock.sock"
HEALTH_DIR="$RUN_DIR/health"
SERVICES_FILE="$RUN_DIR/services.txt"

section "service registry + backend health"
mkdir -p "$HEALTH_DIR"
: > "$HEALTH_DIR/192.168.204.1_6443"
: > "$HEALTH_DIR/192.168.204.1_6444"
: > "$HEALTH_DIR/192.168.204.1_6445"
cat > "$SERVICES_FILE" <<EOF
# name       vip            policy  backends(ip:port,...)                   expected_peer
svc-a        10.80.0.1:9443 allow   192.168.204.1:6443,192.168.204.1:6444   spiffe://overdrive.test/ns/default/sa/peer-allowed
svc-hs-deny  10.80.0.2:9443 allow   192.168.204.1:6445                      spiffe://overdrive.test/ns/default/sa/peer-denied
svc-denied   10.80.0.9:9443 deny    -                                       spiffe://overdrive.test/ns/default/sa/peer-denied
EOF
echo "services:"; sed 's/^/  /' "$SERVICES_FILE"

section "create tap $TAP"
ip tuntap add dev "$TAP" mode tap && TAP_CREATED=1
ip addr add "$TAP_IP/24" dev "$TAP"; ip link set "$TAP" up
ip -br addr show "$TAP"

section "start capture, backends (peer), agent (relay)"
tcpdump -i "$TAP" -n -s 0 -U -w "$RUN_DIR/tap.pcap" 2> "$RUN_DIR/tcpdump.err" & TCPDUMP_PID=$!
for _ in $(seq 1 100); do grep -q 'listening on' "$RUN_DIR/tcpdump.err" && break; sleep 0.05; done
"$BIN/peer" "$CERTS" "$TAP_IP" "$T0" > "$RUN_DIR/peer.log" 2>&1 & PEER_PID=$!
for _ in $(seq 1 100); do grep -q 'PEER ready' "$RUN_DIR/peer.log" && break; sleep 0.05; done
"$BIN/relay" "${VSOCK_BASE}_${AGENT_PORT}" "$CERTS" "$T0" "$SERVICES_FILE" "$HEALTH_DIR" > "$RUN_DIR/relay.log" 2>&1 & RELAY_PID=$!
for _ in $(seq 1 100); do [[ -S "${VSOCK_BASE}_${AGENT_PORT}" ]] && break; sleep 0.05; done
echo "tcpdump=$TCPDUMP_PID peer=$PEER_PID relay=$RELAY_PID $(since)"

section "firecracker config (igkm_phase=$PHASE)"
cat > "$RUN_DIR/vm.json" <<EOF
{
  "boot-source": { "kernel_image_path": "$KERNEL", "initrd_path": "$INITRD",
    "boot_args": "console=ttyS0 reboot=t panic=1 pci=off i8042.nokbd i8042.noaux random.trust_cpu=on igkm_phase=$PHASE" },
  "drives": [],
  "machine-config": { "vcpu_count": 2, "mem_size_mib": 512 },
  "network-interfaces": [ { "iface_id": "net0", "guest_mac": "$GUEST_MAC", "host_dev_name": "$TAP" } ],
  "vsock": { "vsock_id": "vsock0", "guest_cid": $GUEST_CID, "uds_path": "$VSOCK_BASE" }
}
EOF

# Fire a host-side inbound mTLS caller at the guest server. $1=identity $2=port $3..=extra
dial() { local id="$1" port="$2"; shift 2
  echo "RUNNER: dial inclient id=$id -> $GUEST_IP:$port $* $(since)"
  "$BIN/inclient" "$CERTS" "$id" "$GUEST_IP:$port" "$T0" "$@" >> "$RUN_DIR/inclient.log" 2>&1 &
  local p=$!; IC_PIDS="$IC_PIDS $p"; echo "$p"; }

section "launch"
"$FC" --no-api --config-file "$RUN_DIR/vm.json" --id igkmd-mesh \
  > "$RUN_DIR/serial.log" 2> "$RUN_DIR/fc.stderr" < /dev/null & FC_PID=$!
echo "fc pid=$FC_PID $(since)"

# Phase state machine driven by serial markers.
TOGGLED=0; A_DIALED=0; D_DIALED=0; F_DIALED=0; EXIT=""
for _ in $(seq 1 $((BOOT_TIMEOUT_S * 20))); do
  if ! kill -0 "$FC_PID" 2>/dev/null; then wait "$FC_PID"; EXIT=$?; break; fi
  case "$PHASE" in
    regression)
      if [[ "$TOGGLED" -eq 0 ]] && grep -aqF 'PAUSE-FOR-HEALTH-TOGGLE' "$RUN_DIR/serial.log"; then
        echo "RUNNER: toggle backend A unhealthy $(since)"; rm -f "$HEALTH_DIR/$BACKEND_A"; TOGGLED=1
      fi ;;
    accept)
      if [[ "$A_DIALED" -eq 0 ]] && grep -aqF 'SERVER-LISTENING-8443' "$RUN_DIR/serial.log"; then
        IC1=$(dial peer-client 8443); A_DIALED=1
      fi
      if [[ "$A_DIALED" -eq 1 && "$D_DIALED" -eq 0 ]] && ! kill -0 "$IC1" 2>/dev/null; then
        echo "RUNNER: allowed caller finished; dialing DENIED caller $(since)"
        IC2=$(dial peer-client-denied 8443); D_DIALED=1
      fi ;;
    nbaccept)
      if [[ "$F_DIALED" -eq 0 ]] && grep -aqF 'SERVER-LISTENING-8444' "$RUN_DIR/serial.log"; then
        IC1=$(dial peer-client 8444 flood 40000); F_DIALED=1
      fi ;;
    nbconnect) : ;;  # guest-initiated; nothing to dial
  esac
  sleep 0.05
done
[[ -n "$EXIT" ]] && { echo "fc exited on its own exit=$EXIT $(since)"; FC_PID=""; } || { echo "fc timeout -> TERM"; stop_pid "$FC_PID" TERM; FC_PID=""; }
sleep 1.5
for p in $IC_PIDS; do stop_pid "$p" TERM; done; IC_PIDS=""
stop_pid "$TCPDUMP_PID" INT; TCPDUMP_PID=""
stop_pid "$PEER_PID" TERM; PEER_PID=""

section "guest serial"
grep -a . "$RUN_DIR/serial.log"
section "relay log (agent: OPEN/ACCEPT/RESOLVE, handshakes, custody)"; cat "$RUN_DIR/relay.log"
section "peer log (backends)"; cat "$RUN_DIR/peer.log"
section "inclient log (inbound host callers)"; cat "$RUN_DIR/inclient.log" 2>/dev/null || echo "(no inbound dials this phase)"
section "tap capture scan"
ls -l "$RUN_DIR/tap.pcap"; sha256sum "$RUN_DIR/tap.pcap"
python3 "$INC/pcap_scan.py" "$RUN_DIR/tap.pcap" "$T0" 2>&1 || echo "(pcap_scan failed)"
section "plaintext marker scan on the WIRE (mesh app markers must be 0x; :5001 PT may appear)"
for s in IGKM-E-REQI IGKM-E-RESP IGKM-E-REQN IGKM-E-FLOOD IGKM-D-REQ-PT from-guest-server from-A from-B PLAINTEXT-IN-GUEST; do
  n=$(strings "$RUN_DIR/tap.pcap" 2>/dev/null | grep -c "$s")
  echo "  wire occurrences of '$s' : $n"
done
section "custody (SVID key must never cross vsock)"
grep -h 'custody:' "$RUN_DIR/relay.log" || true
section "verdict markers"
grep -aE 'VERDICT|SERVER-LISTENING|HANDSHAKE DONE|SERVER-IDENTITY|RESOLVE-MATCH|DENY|EPOLLOUT|SO_ERROR|EINPROGRESS|ET edge|RECEIVED total|background handshake|PLAINTEXT-IN-GUEST|connect .* (returned|FAILED|rc=)' "$RUN_DIR/serial.log" || true
grep -aE 'rx OPEN|rx ACCEPT|rx RESOLVE|handshake complete|policy (ALLOW|deny)|vsock tx RESOLVED' "$RUN_DIR/relay.log" 2>/dev/null || true
section "tap capture (base64 for the evidence tree)"
echo "----- BEGIN tap.pcap base64 -----"
base64 -w 76 "$RUN_DIR/tap.pcap"
echo "----- END tap.pcap base64 -----"
exit 0
