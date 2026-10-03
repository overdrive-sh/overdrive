#!/usr/bin/env bash
# Spike B boot run (root via `cargo xtask metal run`): the in-guest mTLS hook in
# a Unikraft guest on Firecracker v1.17.0, against a host rustls peer and a host
# rustls relay stub reached over vsock.
#
#   run-boot-fc.sh [PRESET]      PRESET: mesh (default)
#
# Sequence: create tap igkmd0 -> tcpdump on it -> peer (TLS :6443-6446, plain
# :5001 on 192.168.203.1) -> relay (Unix listener <uds_path>_7100) -> boot the
# guest -> when the peer has answered the client-first case's first request AND
# the relay has finished that connection, SIGKILL the relay (the guest is then
# sleeping 8 s on its open connection) -> wait for the guest to power off ->
# stop capture, print every log, scan the capture, clean up, diff host state.
#
# Host footprint (all removed on exit): one tap, one firecracker (no jailer, no
# API socket), the peer, the relay, one tcpdump, a per-run directory. Does NOT
# touch the host kernel (no modules, no sysctls) or any other networking.
set -uo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRATCH="$HOME/igkm-spike-a"
readonly PRESET="${1:-mesh}"
readonly FC_BIN="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
readonly KERNEL="$SCRATCH/unikraft/out-igkmd/igkmd_fc-x86_64"
readonly BIN="$SCRATCH/igkmd-host-target/release"
readonly CERTS="$INCREMENT/out/certs"
readonly TAP=igkmd0
readonly TAP_IP=192.168.203.1
readonly GUEST_IP=192.168.203.2
readonly GUEST_MAC=52:54:00:1a:03:04
readonly GUEST_CID=3
readonly AGENT_PORT=7100
readonly BOOT_TIMEOUT_S=90

case "$PRESET" in
  mesh) BOOT_ARGS="igkmd netdev.ip=$GUEST_IP/24:$TAP_IP mtlsguard.mesh=$TAP_IP:6443-6446 mtlsguard.mesh_in=$GUEST_IP:7443 mtlsguard.agent_port=$AGENT_PORT -- run" ;;
  *) echo "unknown PRESET $PRESET"; exit 2 ;;
esac

RUN_DIR="$SCRATCH/runs-igkmd/$(date -u +%Y%m%dT%H%M%SZ)-$$"
readonly RUN_DIR
FC_PID="" ; RELAY_PID="" ; PEER_PID="" ; TCPDUMP_PID="" ; INCLIENT_PID="" ; TAP_CREATED=0

section() { printf '\n===== %s =====\n' "$*"; }
snap_links() { ip -o link show | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | sort; }
snap_fc() { pgrep -a firecracker | sort || true; }
T0="$(date +%s.%N)"
since() { python3 -c "import time; print(f't+{time.time()-$T0:.6f}s')"; }

stop_pid() { # pid signal name
  local p="$1" sig="$2" name="$3"
  [[ -n "$p" ]] || return 0
  if kill -0 "$p" 2>/dev/null; then
    echo "$name pid $p still running -> SIG$sig"
    kill -"$sig" "$p"
    for _ in $(seq 1 50); do kill -0 "$p" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$p" 2>/dev/null && { echo "-> SIGKILL"; kill -KILL "$p"; }
  fi
  wait "$p" 2>/dev/null
}

cleanup() {
  set +e
  section "cleanup"
  stop_pid "$FC_PID" TERM firecracker; FC_PID=""
  stop_pid "$RELAY_PID" TERM relay; RELAY_PID=""
  stop_pid "$PEER_PID" TERM peer; PEER_PID=""
  stop_pid "$TCPDUMP_PID" INT tcpdump; TCPDUMP_PID=""
  stop_pid "$INCLIENT_PID" TERM inclient; INCLIENT_PID=""
  if [[ "$TAP_CREATED" -eq 1 ]] && ip link show "$TAP" >/dev/null 2>&1; then
    echo "deleting tap $TAP"; ip link del "$TAP"
  fi
  [[ -d "$RUN_DIR" && "$RUN_DIR" == "$SCRATCH"/runs-igkmd/* ]] && rm -rf -- "$RUN_DIR"
  rmdir "$SCRATCH/runs-igkmd" 2>/dev/null
  section "post-run host state (must match pre-run)"
  AFTER_LINKS="$(snap_links)"; AFTER_FC="$(snap_fc)"
  echo "links added:   $(comm -13 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "links removed: $(comm -23 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "firecracker processes before: [${BEFORE_FC}] after: [${AFTER_FC}]"
  echo "tap $TAP present: $(ip link show "$TAP" >/dev/null 2>&1 && echo yes || echo no)"
  echo "192.168.203.x on host: $(ip -4 -br addr | grep -c '192\.168\.203\.')"
  echo "listeners on 5001/6443-6446: [$(ss -Htlnp 2>/dev/null | grep -E ':(5001|6443|6444|6445|6446)\b' | tr '\n' ';')]"
  echo "igkmd processes left: [$(pgrep -af 'igkmd-host-target|tcpdump -i igkmd0' | tr '\n' ';')]"
  echo "run dir present: $([[ -e "$RUN_DIR" ]] && echo yes || echo no)"
}

section "pre-run host state"
BEFORE_LINKS="$(snap_links)"
BEFORE_FC="$(snap_fc)"
echo "links: $(echo "$BEFORE_LINKS" | tr '\n' ' ')"
echo "firecracker processes: [${BEFORE_FC}]"
if ip link show "$TAP" >/dev/null 2>&1; then echo "FATAL: tap $TAP already exists"; exit 2; fi
ip -4 addr show | grep -q "192\.168\.203\." && { echo "FATAL: 192.168.203.0/24 already in use"; exit 2; }
ss -Htln | grep -qE ':(5001|6443|6444|6445|6446)\b' && { echo "FATAL: a test port is in use"; exit 2; }
trap cleanup EXIT HUP INT TERM

section "versions / artifacts"
uname -r
"$FC_BIN" --version 2>&1 | head -1
git -C "$SCRATCH/unikraft/unikraft-igkmd" rev-parse HEAD
git -C "$SCRATCH/unikraft/unikraft-igkmd" diff --stat
git -C "$SCRATCH/unikraft/lib-lwip" rev-parse HEAD
sha256sum "$FC_BIN" "$KERNEL" "$BIN/peer" "$BIN/relay" "$BIN/inclient" "$INCREMENT"/patches/*.patch
echo "preset=$PRESET boot_args=\"$BOOT_ARGS\""

mkdir -p "$RUN_DIR"
readonly VSOCK_BASE="$RUN_DIR/vsock.sock"

section "create tap $TAP (the only host network change)"
ip tuntap add dev "$TAP" mode tap && TAP_CREATED=1
ip addr add "$TAP_IP/24" dev "$TAP"
ip link set "$TAP" up
ip -br addr show "$TAP"

section "start capture, peer, relay"
# -Z root: do not drop to the tcpdump user; -w - to stdout so neither the
# privilege drop nor the AppArmor path rules apply to the output file.
tcpdump -i "$TAP" -n -s 0 -U -Z root -w - > "$RUN_DIR/tap.pcap" 2> "$RUN_DIR/tcpdump.err" &
TCPDUMP_PID=$!
for _ in $(seq 1 100); do grep -q "listening on" "$RUN_DIR/tcpdump.err" && break; sleep 0.05; done
cat "$RUN_DIR/tcpdump.err"
"$BIN/peer" "$CERTS" "$TAP_IP" "$T0" > "$RUN_DIR/peer.log" 2>&1 &
PEER_PID=$!
for _ in $(seq 1 100); do grep -q "PEER ready" "$RUN_DIR/peer.log" && break; sleep 0.05; done
"$BIN/relay" "${VSOCK_BASE}_${AGENT_PORT}" "$CERTS" "$T0" > "$RUN_DIR/relay.log" 2>&1 &
RELAY_PID=$!
for _ in $(seq 1 100); do [[ -S "${VSOCK_BASE}_${AGENT_PORT}" ]] && break; sleep 0.05; done
echo "tcpdump pid=$TCPDUMP_PID peer pid=$PEER_PID relay pid=$RELAY_PID $(since)"
ss -Htlnp | grep -E ':(5001|6443|6444|6445|6446)\b'
ss -Hxlp | grep -F "${VSOCK_BASE}_${AGENT_PORT}"

section "firecracker config"
cat > "$RUN_DIR/vm.json" <<EOF
{
  "boot-source": {
    "kernel_image_path": "$KERNEL",
    "boot_args": "$BOOT_ARGS"
  },
  "drives": [],
  "machine-config": { "vcpu_count": 1, "mem_size_mib": 512 },
  "network-interfaces": [
    { "iface_id": "net0", "guest_mac": "$GUEST_MAC", "host_dev_name": "$TAP" }
  ],
  "vsock": { "vsock_id": "vsock0", "guest_cid": $GUEST_CID, "uds_path": "$VSOCK_BASE" },
  "logger": { "log_path": "$RUN_DIR/fc.log", "level": "Info", "show_level": true, "show_log_origin": true },
  "metrics": { "metrics_path": "$RUN_DIR/fc.metrics" }
}
EOF
cat "$RUN_DIR/vm.json"
python3 -m json.tool "$RUN_DIR/vm.json" > /dev/null || { echo "FATAL: vm.json is not valid JSON"; exit 2; }
touch "$RUN_DIR/fc.log" "$RUN_DIR/fc.metrics"

section "launch"
"$FC_BIN" --no-api --config-file "$RUN_DIR/vm.json" --id igkm-d > "$RUN_DIR/serial.log" 2> "$RUN_DIR/fc.stderr" < /dev/null &
FC_PID=$!
echo "firecracker pid=$FC_PID $(since)"

FC_EXIT=""; KILLED=0; INBOUND=0
for _ in $(seq 1 $((BOOT_TIMEOUT_S * 20))); do
  if ! kill -0 "$FC_PID" 2>/dev/null; then wait "$FC_PID"; FC_EXIT=$?; break; fi
  if [[ "$INBOUND" -eq 0 ]] && grep -aq 'READY-FOR-INBOUND' "$RUN_DIR/serial.log"; then
    echo "RUNNER: guest listening on :7443 -> inbound mTLS callers (peer-client, then peer-client-denied) $(since)"
    ( "$BIN/inclient" "$CERTS" peer-client "$GUEST_IP:7443" "$T0"
      "$BIN/inclient" "$CERTS" peer-client-denied "$GUEST_IP:7443" "$T0" ) > "$RUN_DIR/inclient.log" 2>&1 &
    INCLIENT_PID=$!; INBOUND=1
  fi
  if [[ "$KILLED" -eq 0 ]] && grep -q 'PEER\[6443#1\] tx plaintext line 1 ' "$RUN_DIR/peer.log"; then
    tag="$(grep -oE 'RELAY\[c[0-9]+\] vsock rx OPEN payload=7 dst=192\.168\.203\.1:6443 ' "$RUN_DIR/relay.log" | head -1 | grep -oE 'c[0-9]+' | head -1)"
    if [[ -n "$tag" ]] && grep -q "RELAY\[$tag\] custody:" "$RUN_DIR/relay.log"; then
      echo "RUNNER: client-first exchange 1 answered and relay connection $tag finished -> SIGKILL relay pid $RELAY_PID $(since)"
      kill -KILL "$RELAY_PID"; wait "$RELAY_PID" 2>/dev/null
      echo "RUNNER: relay killed $(since); kill -0: $(kill -0 "$RELAY_PID" 2>/dev/null && echo alive || echo gone)"
      echo "RUNNER: relay processes now: [$(pgrep -af "$BIN/relay" | tr '\n' ';')]"
      echo "RUNNER: unix listeners on the agent path now: [$(ss -Hxlp | grep -F "${VSOCK_BASE}_${AGENT_PORT}" | tr '\n' ';')]"
      echo "RUNNER: stale socket file left by the killed relay: $(ls -l "${VSOCK_BASE}_${AGENT_PORT}" 2>&1)"
      RELAY_PID=""; KILLED=1
    fi
  fi
  sleep 0.05
done
if [[ -n "$FC_EXIT" ]]; then
  echo "firecracker exited on its own: exit=$FC_EXIT $(since)"
  FC_PID=""
else
  echo "firecracker still running after ${BOOT_TIMEOUT_S}s -> SIGTERM $(since)"
  stop_pid "$FC_PID" TERM firecracker; FC_PID=""
fi
[[ "$KILLED" -eq 1 ]] || echo "RUNNER: relay was NOT killed (trigger never seen)"
sleep 1
stop_pid "$TCPDUMP_PID" INT tcpdump; TCPDUMP_PID=""
stop_pid "$PEER_PID" TERM peer; PEER_PID=""
stop_pid "$RELAY_PID" TERM relay; RELAY_PID=""

stop_pid "$INCLIENT_PID" TERM inclient; INCLIENT_PID=""
section "inbound caller log"
cat "$RUN_DIR/inclient.log" 2>&1
section "relay log"
cat "$RUN_DIR/relay.log"
section "peer log"
cat "$RUN_DIR/peer.log"
section "guest serial console (Unikraft)"
cat "$RUN_DIR/serial.log"
section "firecracker stderr"
cat "$RUN_DIR/fc.stderr"
section "firecracker log"
cat "$RUN_DIR/fc.log"
section "firecracker metrics (non-zero vsock/net/i8042 counters)"
python3 - "$RUN_DIR/fc.metrics" <<'PY' 2>&1 || echo "(no metrics)"
import json, sys
def leaves(prefix, node):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from leaves(f"{prefix}.{k}" if prefix else k, v)
    else:
        yield prefix, node
for n, line in enumerate(open(sys.argv[1]), 1):
    line = line.strip()
    if not line:
        continue
    for k, v in leaves("", json.loads(line)):
        if k.split(".")[0] in ("vsock", "net", "i8042") and v not in (0, None, False, ""):
            print(f"   line{n} {k} = {v}")
PY
section "tcpdump"
cat "$RUN_DIR/tcpdump.err"
ls -l "$RUN_DIR/tap.pcap"
sha256sum "$RUN_DIR/tap.pcap"
section "tap capture scan"
python3 "$INCREMENT/pcap_scan.py" "$RUN_DIR/tap.pcap"
section "tap capture (base64, for the evidence tree)"
echo "----- BEGIN tap.pcap base64 -----"
base64 -w 76 "$RUN_DIR/tap.pcap"
echo "----- END tap.pcap base64 -----"
section "run verdict"
grep -ah "VERDICT" "$RUN_DIR/serial.log"
grep -h "custody:" "$RUN_DIR/relay.log"
exit 0
