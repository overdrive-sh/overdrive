#!/usr/bin/env bash
# Spike A3 boot run (root via `cargo xtask metal run`): boot the native Unikraft
# image under Firecracker with direct kernel boot (no disk), virtio-net
# (pre-created tap) and virtio-vsock (Firecracker Unix-socket muxer).
#
#   run-boot-fc.sh FIRECRACKER_BIN KERNEL PRESET
#
# PRESET selects `boot-source.boot_args` (named presets, so no quoting crosses
# the ssh boundary):
#   uk         "igkmc netdev.ip=192.168.203.2/24:192.168.203.1 -- probe"
#              (argv[0] token, then Unikraft libparams, then ` -- ` + one app arg)
#   fcdefault  boot_args omitted: Firecracker's DEFAULT_KERNEL_CMDLINE (no `--`)
#   noargv0    "netdev.ip=192.168.203.2/24:192.168.203.1 -- probe"
#              (libparam first, no program-name token)
#
# Host footprint (all removed on exit, verified by before/after snapshots):
#   - one tap device (TAP below, created with `ip tuntap add`, deleted in cleanup)
#   - one firecracker process (no jailer, no API socket: --no-api --config-file)
#   - one python3 listener
#   - Unix sockets, the JSON config and logs under a per-run dir
# Does NOT touch the host kernel (no modules, no sysctls) or any other networking.
set -uo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRATCH="$HOME/igkm-spike-a"
readonly FC_BIN="${1:?usage: run-boot-fc.sh FIRECRACKER_BIN KERNEL PRESET}"
readonly KERNEL="${2:?usage: run-boot-fc.sh FIRECRACKER_BIN KERNEL PRESET}"
readonly PRESET="${3:?usage: run-boot-fc.sh FIRECRACKER_BIN KERNEL PRESET}"
readonly TAP=igkmc0
readonly TAP_IP=192.168.203.1
readonly GUEST_IP=192.168.203.2
readonly GUEST_MAC=52:54:00:1a:03:02
readonly GUEST_CID=3
readonly VSOCK_PORT=5000
readonly NET_PORT=5001
readonly BOOT_TIMEOUT_S=60

case "$PRESET" in
  uk)        BOOT_ARGS_JSON=",
    \"boot_args\": \"igkmc netdev.ip=$GUEST_IP/24:$TAP_IP -- probe\"" ;;
  fcdefault) BOOT_ARGS_JSON="" ;;
  noargv0)   BOOT_ARGS_JSON=",
    \"boot_args\": \"netdev.ip=$GUEST_IP/24:$TAP_IP -- probe\"" ;;
  *) echo "unknown PRESET $PRESET"; exit 2 ;;
esac

RUN_DIR="$SCRATCH/runs-ukfc/$(date -u +%Y%m%dT%H%M%SZ)-$$"
readonly RUN_DIR
FC_PID=""
LISTENER_PID=""
TAP_CREATED=0

section() { printf '\n===== %s =====\n' "$*"; }
snap_links() { ip -o link show | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | sort; }
snap_fc() { pgrep -a firecracker | sort || true; }

cleanup() {
  set +e
  section "cleanup"
  if [[ -n "$FC_PID" ]] && kill -0 "$FC_PID" 2>/dev/null; then
    echo "firecracker pid $FC_PID still running -> SIGTERM"
    kill -TERM "$FC_PID"
    for _ in $(seq 1 50); do kill -0 "$FC_PID" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$FC_PID" 2>/dev/null && { echo "-> SIGKILL"; kill -KILL "$FC_PID"; }
    wait "$FC_PID" 2>/dev/null
  fi
  if [[ -n "$LISTENER_PID" ]] && kill -0 "$LISTENER_PID" 2>/dev/null; then
    echo "listener pid $LISTENER_PID still running -> SIGTERM"
    kill -TERM "$LISTENER_PID"; wait "$LISTENER_PID" 2>/dev/null
  fi
  if [[ "$TAP_CREATED" -eq 1 ]] && ip link show "$TAP" >/dev/null 2>&1; then
    echo "deleting tap $TAP"
    ip link del "$TAP"
  fi
  [[ -d "$RUN_DIR" && "$RUN_DIR" == "$SCRATCH"/runs-ukfc/* ]] && rm -rf -- "$RUN_DIR"
  rmdir "$SCRATCH/runs-ukfc" 2>/dev/null
  section "post-run host state (must match pre-run)"
  AFTER_LINKS="$(snap_links)"; AFTER_FC="$(snap_fc)"
  echo "links added:   $(comm -13 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "links removed: $(comm -23 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "firecracker processes before: [${BEFORE_FC}] after: [${AFTER_FC}]"
  echo "tap $TAP present: $(ip link show "$TAP" >/dev/null 2>&1 && echo yes || echo no)"
  echo "192.168.203.x on host: $(ip -4 -br addr | grep -c '192\.168\.203\.')"
  echo "run dir present: $([[ -e "$RUN_DIR" ]] && echo yes || echo no)"
  echo "leftover listener/firecracker pids: $(pgrep -f "$RUN_DIR" | tr '\n' ' ')"
}

section "pre-run host state"
BEFORE_LINKS="$(snap_links)"
BEFORE_FC="$(snap_fc)"
echo "links: $(echo "$BEFORE_LINKS" | tr '\n' ' ')"
echo "firecracker processes: [${BEFORE_FC}]"
if ip link show "$TAP" >/dev/null 2>&1; then echo "FATAL: tap $TAP already exists"; exit 2; fi
ip -4 addr show | grep -q "192\.168\.203\." && { echo "FATAL: 192.168.203.0/24 already in use"; exit 2; }
trap cleanup EXIT HUP INT TERM

section "versions / artifacts"
uname -r
"$FC_BIN" --version
git -C "$SCRATCH/unikraft/unikraft" rev-parse HEAD
git -C "$SCRATCH/unikraft/lib-lwip" rev-parse HEAD
sha256sum "$FC_BIN" "$KERNEL"
find "$SCRATCH/unikraft/build-igkmc/liblwip" -maxdepth 1 -type f -name '*.zip' -exec sha256sum {} \; 2>/dev/null
echo "--- kernel ELF entry and notes (no Xen note = no PVH entry)"
readelf -h "$KERNEL" | grep 'Entry point'
echo "Xen notes: $(readelf -n "$KERNEL" | grep -c Xen)"
echo "preset=$PRESET"

mkdir -p "$RUN_DIR"
readonly VSOCK_BASE="$RUN_DIR/vsock.sock"

section "create tap $TAP (the only host network change)"
ip tuntap add dev "$TAP" mode tap && TAP_CREATED=1
ip addr add "$TAP_IP/24" dev "$TAP"
ip link set "$TAP" up
ip -br addr show "$TAP"

section "firecracker config"
cat > "$RUN_DIR/vm.json" <<EOF
{
  "boot-source": {
    "kernel_image_path": "$KERNEL"$BOOT_ARGS_JSON
  },
  "drives": [],
  "machine-config": {
    "vcpu_count": 1,
    "mem_size_mib": 512
  },
  "network-interfaces": [
    {
      "iface_id": "net0",
      "guest_mac": "$GUEST_MAC",
      "host_dev_name": "$TAP"
    }
  ],
  "vsock": {
    "vsock_id": "vsock0",
    "guest_cid": $GUEST_CID,
    "uds_path": "$VSOCK_BASE"
  },
  "logger": {
    "log_path": "$RUN_DIR/fc.log",
    "level": "Debug",
    "show_level": true,
    "show_log_origin": true
  },
  "metrics": {
    "metrics_path": "$RUN_DIR/fc.metrics"
  }
}
EOF
cat "$RUN_DIR/vm.json"
python3 -m json.tool "$RUN_DIR/vm.json" > /dev/null || { echo "FATAL: vm.json is not valid JSON"; exit 2; }
# Firecracker <= 1.11 opens log/metrics paths without O_CREAT (Spike A2 run 0006); pre-create both.
touch "$RUN_DIR/fc.log" "$RUN_DIR/fc.metrics"

section "launch"
T0="$(date +%s.%N)"
python3 "$INCREMENT/host_listener.py" "${VSOCK_BASE}_${VSOCK_PORT}" "$TAP_IP" "$NET_PORT" "$T0" "$BOOT_TIMEOUT_S" \
  > "$RUN_DIR/listener.log" 2>&1 &
LISTENER_PID=$!
for _ in $(seq 1 50); do [[ -S "${VSOCK_BASE}_${VSOCK_PORT}" ]] && break; sleep 0.05; done

FC_CMD=("$FC_BIN" --no-api --config-file "$RUN_DIR/vm.json" --id igkm-c)
printf 'Firecracker command: '; printf '%q ' "${FC_CMD[@]}"; echo
T_FC="$(date +%s.%N)"
"${FC_CMD[@]}" > "$RUN_DIR/serial.log" 2> "$RUN_DIR/fc.stderr" < /dev/null &
FC_PID=$!
echo "firecracker pid=$FC_PID listener pid=$LISTENER_PID T0=$T0 firecracker-start=t+$(python3 -c "print(f'{$T_FC-$T0:.6f}')")s"

FC_EXIT=""
for _ in $(seq 1 $((BOOT_TIMEOUT_S * 20))); do
  if ! kill -0 "$FC_PID" 2>/dev/null; then wait "$FC_PID"; FC_EXIT=$?; break; fi
  sleep 0.05
done
T_END="$(date +%s.%N)"
if [[ -n "$FC_EXIT" ]]; then
  echo "firecracker exited on its own: exit=$FC_EXIT at t+$(python3 -c "print(f'{$T_END-$T0:.3f}')")s (<=0.05s poll)"
  FC_PID=""
else
  echo "firecracker still running after ${BOOT_TIMEOUT_S}s (guest did not power off) -> SIGTERM now, before reading logs"
  kill -TERM "$FC_PID"
  for _ in $(seq 1 50); do kill -0 "$FC_PID" 2>/dev/null || break; sleep 0.1; done
  kill -0 "$FC_PID" 2>/dev/null && { echo "-> SIGKILL"; kill -KILL "$FC_PID"; }
  wait "$FC_PID"; echo "firecracker exit after signal: $?"
  FC_PID=""
fi
# A leg that never connects keeps the listener waiting until its own deadline
# (BOOT_TIMEOUT_S after T0); it then prints its FAIL verdicts and exits.
wait "$LISTENER_PID"; LISTENER_EXIT=$?; LISTENER_PID=""
echo "listener exit=$LISTENER_EXIT"

section "host listener log"
cat "$RUN_DIR/listener.log"
section "guest serial console (Unikraft, = firecracker stdout)"
cat "$RUN_DIR/serial.log" 2>&1
section "firecracker stderr"
cat "$RUN_DIR/fc.stderr" 2>&1
section "firecracker log (Debug)"
cat "$RUN_DIR/fc.log" 2>&1
section "firecracker metrics (every flushed line; non-zero counters only; counters are per-flush deltas)"
python3 - "$RUN_DIR/fc.metrics" <<'PY' 2>&1 || echo "(no metrics)"
import json, sys
def leaves(prefix, node):
    if isinstance(node, dict):
        for k, v in node.items():
            yield from leaves(f"{prefix}.{k}" if prefix else k, v)
    else:
        yield prefix, node
with open(sys.argv[1]) as f:
    for n, line in enumerate(f, 1):
        line = line.strip()
        if not line:
            continue
        doc = json.loads(line)
        print(f"-- metrics line {n} utc_timestamp_ms={doc.get('utc_timestamp_ms')}")
        for k, v in leaves("", doc):
            if k != "utc_timestamp_ms" and v not in (0, None, False, ""):
                print(f"   {k} = {v}")
PY

section "run verdict"
grep -h "VERDICT" "$RUN_DIR/listener.log" "$RUN_DIR/serial.log" 2>/dev/null
exit "$LISTENER_EXIT"
