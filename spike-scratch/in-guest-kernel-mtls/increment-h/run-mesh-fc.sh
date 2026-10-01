#!/usr/bin/env bash
# Spike C re-run (GH #303, increment-h) BOOT phase. Runs as ROOT (cargo xtask
# metal run, default sudo). It does NOT build anything -- build-all.sh must have
# run first as the login user (--no-sudo) so the artifacts are login-user-owned
# in the unsynced scratch tree. This script only: stages the stock kernel, wires
# a uniquely named tap + tcpdump + host peer + host relay, boots Firecracker,
# kills the relay mid-run to prove kTLS survives on the workload socket, scans
# the tap capture, emits the pcap as base64 for the evidence tree, and cleans up
# every host object, proving residual state.
set -uo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRATCH="$HOME/igkm-spike-a"
FC="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
BIN="$SCRATCH/igkmh-host-target/release"
OUT="$SCRATCH/igkmh/out"
KDIR="$SCRATCH/igkmh/kernel"
CERTS="$OUT/certs"
KREL="$(uname -r)"
TAP="igkh$(date -u +%d%H%M%S)"
TAP_IP=192.168.204.1
GUEST_IP=192.168.204.2
GUEST_MAC=52:54:00:1a:05:07
GUEST_CID=3
AGENT_PORT=7100
BOOT_TIMEOUT_S=70
RUN_DIR="$SCRATCH/runs-igkmh/$(date -u +%Y%m%dT%H%M%SZ)-$$"
FC_PID=""; RELAY_PID=""; PEER_PID=""; TCPDUMP_PID=""; TAP_CREATED=0
T0="$(date +%s.%N)"
since() { python3 -c "import time; print(f't+{time.time()-$T0:.3f}s')"; }
section() { printf '\n===== %s =====\n' "$*"; }
snap_links() { ip -o link show | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | sort; }

stop_pid() { local p="$1" sig="$2" nm="$3"; [[ -n "$p" ]] || return 0
  if kill -0 "$p" 2>/dev/null; then kill -"$sig" "$p"
    for _ in $(seq 1 40); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$p" 2>/dev/null && kill -KILL "$p"; fi; wait "$p" 2>/dev/null; }
cleanup() { set +e; section "cleanup"
  stop_pid "$FC_PID" TERM fc; FC_PID=""
  stop_pid "$RELAY_PID" KILL relay; RELAY_PID=""
  stop_pid "$PEER_PID" TERM peer; PEER_PID=""
  stop_pid "$TCPDUMP_PID" INT tcpdump; TCPDUMP_PID=""
  [[ "$TAP_CREATED" -eq 1 ]] && ip link show "$TAP" >/dev/null 2>&1 && { echo "del tap $TAP"; ip link del "$TAP"; }
  [[ -d "$RUN_DIR" && "$RUN_DIR" == "$SCRATCH"/runs-igkmh/* ]] && rm -rf -- "$RUN_DIR"
  rmdir "$SCRATCH/runs-igkmh" 2>/dev/null
  section "post-run host state"
  echo "tap $TAP present: $(ip link show "$TAP" >/dev/null 2>&1 && echo yes || echo no)"
  echo "192.168.204.x on host: $(ip -4 -br addr | grep -c '192\.168\.204\.')"
  echo "firecracker procs: [$(pgrep -a firecracker | tr '\n' ';')]"
  echo "peer/relay procs: [$(pgrep -af "$BIN/(peer|relay)" | tr '\n' ';')]"
  echo "tcpdump procs: [$(pgrep -a tcpdump | tr '\n' ';')]"
  echo "run dir present: $([[ -e "$RUN_DIR" ]] && echo yes || echo no)"; }

section "preconditions (artifacts must already be built by build-all.sh --no-sudo)"
for f in "$FC" "$OUT/module/igkmh_mtls.ko" "$OUT/initramfs-mesh.cpio.gz" "$OUT/bin/igkmh_app" \
         "$BIN/peer" "$BIN/relay" "$CERTS/client.pem" "$CERTS/peer-allowed.pem"; do
  [[ -e "$f" ]] || { echo "FATAL: missing prebuilt artifact: $f (run build-all.sh --no-sudo first)"; exit 2; }
done
echo "all prebuilt artifacts present"

section "pre-run host state"
BEFORE_LINKS="$(snap_links)"; echo "links: $(echo "$BEFORE_LINKS" | tr '\n' ' ')"
ip link show "$TAP" >/dev/null 2>&1 && { echo "FATAL: tap $TAP exists"; exit 2; }
ip -4 addr show | grep -q '192\.168\.204\.' && { echo "FATAL: 192.168.204.0/24 in use"; exit 2; }
ss -Htln | grep -qE ':(5001|644[3-9]|645[012])\b' && { echo "FATAL: a test port is in use"; exit 2; }
trap cleanup EXIT HUP INT TERM

section "stage stock kernel (root reads /boot/vmlinuz; guest kernel == host kernel)"
[[ -f "$KDIR/vmlinuz-host" ]] || install -D -m0644 /boot/vmlinuz-"$KREL" "$KDIR/vmlinuz-host"
KERNEL="$KDIR/vmlinuz-host"; INITRD="$OUT/initramfs-mesh.cpio.gz"

section "artifacts"
echo "host uname -r: $KREL"; "$FC" --version | head -1
sha256sum "$KERNEL" "$INITRD" "$OUT/module/igkmh_mtls.ko" "$OUT/bin/igkmh_app" "$BIN/peer" "$BIN/relay"
echo "module vermagic: $(modinfo -F vermagic "$OUT/module/igkmh_mtls.ko" 2>/dev/null)"

mkdir -p "$RUN_DIR"
VSOCK_BASE="$RUN_DIR/vsock.sock"

section "create tap $TAP (only host network change)"
ip tuntap add dev "$TAP" mode tap && TAP_CREATED=1
ip addr add "$TAP_IP/24" dev "$TAP"; ip link set "$TAP" up
ip -br addr show "$TAP"

section "start capture, peer, relay"
tcpdump -i "$TAP" -n -s 0 -U -w "$RUN_DIR/tap.pcap" 2> "$RUN_DIR/tcpdump.err" & TCPDUMP_PID=$!
for _ in $(seq 1 100); do grep -q 'listening on' "$RUN_DIR/tcpdump.err" && break; sleep 0.05; done
"$BIN/peer" "$CERTS" "$TAP_IP" "$T0" > "$RUN_DIR/peer.log" 2>&1 & PEER_PID=$!
for _ in $(seq 1 100); do grep -q 'PEER ready' "$RUN_DIR/peer.log" && break; sleep 0.05; done
"$BIN/relay" "${VSOCK_BASE}_${AGENT_PORT}" "$CERTS" "$T0" > "$RUN_DIR/relay.log" 2>&1 & RELAY_PID=$!
for _ in $(seq 1 100); do [[ -S "${VSOCK_BASE}_${AGENT_PORT}" ]] && break; sleep 0.05; done
echo "tcpdump=$TCPDUMP_PID peer=$PEER_PID relay=$RELAY_PID $(since)"
ss -Hxlp | grep -F "${VSOCK_BASE}_${AGENT_PORT}" || true

section "firecracker config"
cat > "$RUN_DIR/vm.json" <<EOF
{
  "boot-source": { "kernel_image_path": "$KERNEL", "initrd_path": "$INITRD",
    "boot_args": "console=ttyS0 reboot=t panic=1 pci=off i8042.nokbd i8042.noaux random.trust_cpu=on" },
  "drives": [],
  "machine-config": { "vcpu_count": 1, "mem_size_mib": 512 },
  "network-interfaces": [ { "iface_id": "net0", "guest_mac": "$GUEST_MAC", "host_dev_name": "$TAP" } ],
  "vsock": { "vsock_id": "vsock0", "guest_cid": $GUEST_CID, "uds_path": "$VSOCK_BASE" }
}
EOF
cat "$RUN_DIR/vm.json"

section "launch"
"$FC" --no-api --config-file "$RUN_DIR/vm.json" --id igkmh-mesh \
  > "$RUN_DIR/serial.log" 2> "$RUN_DIR/fc.stderr" < /dev/null & FC_PID=$!
echo "fc pid=$FC_PID $(since)"
KILLED=0; EXIT=""
for _ in $(seq 1 $((BOOT_TIMEOUT_S * 20))); do
  if ! kill -0 "$FC_PID" 2>/dev/null; then wait "$FC_PID"; EXIT=$?; break; fi
  if [[ "$KILLED" -eq 0 ]] && grep -aqF 'PAUSE-FOR-RELAY-KILL' "$RUN_DIR/serial.log"; then
    echo "RUNNER: guest paused after exchange-1 -> SIGKILL relay pid $RELAY_PID (prove kTLS survives) $(since)"
    kill -KILL "$RELAY_PID" 2>/dev/null; wait "$RELAY_PID" 2>/dev/null; RELAY_PID=""; KILLED=1
    echo "RUNNER: relay killed; unix listeners now: [$(ss -Hxlp | grep -F "${VSOCK_BASE}_${AGENT_PORT}" | tr '\n' ';')]"
  fi
  sleep 0.05
done
[[ -n "$EXIT" ]] && { echo "fc exited on its own exit=$EXIT $(since)"; FC_PID=""; } || { echo "fc timeout -> TERM"; stop_pid "$FC_PID" TERM fc; FC_PID=""; }
[[ "$KILLED" -eq 1 ]] || echo "RUNNER: relay was NOT killed (PAUSE marker never seen)"
sleep 0.5
stop_pid "$TCPDUMP_PID" INT tcpdump; TCPDUMP_PID=""
stop_pid "$PEER_PID" TERM peer; PEER_PID=""

section "guest serial"
grep -a . "$RUN_DIR/serial.log"
section "relay log"; cat "$RUN_DIR/relay.log"
section "peer log"; cat "$RUN_DIR/peer.log"
section "tap capture scan (mesh plaintext REQUEST/RESPONSE must be 0x; plaintext OK on :5001)"
ls -l "$RUN_DIR/tap.pcap"; sha256sum "$RUN_DIR/tap.pcap"
python3 "$INC/pcap_scan.py" "$RUN_DIR/tap.pcap" "$T0" 2>&1 || echo "(pcap_scan failed)"
section "plaintext marker scan on the WIRE (my IGKM-H- strings): only the :5001 passthrough may appear"
for s in IGKM-H-REQ-PT IGKM-H-RESP-PT IGKM-H-REQ-SF IGKM-H-REQ-e1 IGKM-H-REQ-c2 IGKM-H-REQ-DR IGKM-H-REQ-A IGKM-H-REQ-B IGKM-H-REQ-DENY; do
  n=$(strings "$RUN_DIR/tap.pcap" 2>/dev/null | grep -c "$s")
  echo "  wire occurrences of '$s' : $n"
done
echo "(expected: PT strings >=1 cleartext on non-mesh :5001; every mesh string = 0, inside kTLS records)"
section "custody (SVID key must never cross vsock)"
grep -h 'custody:' "$RUN_DIR/relay.log" || true
section "verdict markers"
grep -aE 'VERDICT|HANDSHAKE DONE|DENY|PAUSE-FOR|APP: .*read|recvmsg|connect .* (returned|FAILED)' "$RUN_DIR/serial.log" || true
section "tap capture (base64, for the evidence tree -- capture.sh extracts runs/NNNN.tap.pcap)"
echo "----- BEGIN tap.pcap base64 -----"
base64 -w 76 "$RUN_DIR/tap.pcap"
echo "----- END tap.pcap base64 -----"
exit 0
