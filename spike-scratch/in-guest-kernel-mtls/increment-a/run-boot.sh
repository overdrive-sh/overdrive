#!/usr/bin/env bash
# Spike A boot run (root via `cargo xtask metal run`): boot the Nanos image under
# Cloud Hypervisor through PVH direct kernel boot, with virtio-blk root,
# virtio-net (CH-created tap) and virtio-vsock (CH Unix-socket muxer).
#
# Host footprint (all removed on exit, verified by before/after snapshots):
#   - one cloud-hypervisor process, one tap (TAP below, non-persistent: dies
#     with CH), one python3 listener, unix sockets + a disk-image copy under a
#     per-run dir. Does NOT touch the host kernel (no modules, no sysctls).
set -uo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRATCH="$HOME/igkm-spike-a"
# Optional arg: kernel image to boot (default: the unmodified aad473aa build).
readonly KERNEL="${1:-$SCRATCH/nanos/output/platform/pc/bin/kernel.img}"
readonly IMAGE="$SCRATCH/out/nanos-vsock.img"
readonly TAP=igkma0
readonly TAP_IP=192.168.203.1
readonly GUEST_CID=3
readonly VSOCK_PORT=5000
readonly NET_PORT=5001
readonly BOOT_TIMEOUT_S=60

RUN_DIR="$SCRATCH/runs/$(date -u +%Y%m%dT%H%M%SZ)-$$"
readonly RUN_DIR
CH_PID=""
LISTENER_PID=""

section() { printf '\n===== %s =====\n' "$*"; }
snap_links() { ip -o link show | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | sort; }
snap_ch() { pgrep -a cloud-hypervisor | sort || true; }

cleanup() {
  set +e
  section "cleanup"
  if [[ -n "$CH_PID" ]] && kill -0 "$CH_PID" 2>/dev/null; then
    echo "cloud-hypervisor pid $CH_PID still running -> SIGTERM"
    kill -TERM "$CH_PID"
    for _ in $(seq 1 50); do kill -0 "$CH_PID" 2>/dev/null || break; sleep 0.1; done
    kill -0 "$CH_PID" 2>/dev/null && { echo "-> SIGKILL"; kill -KILL "$CH_PID"; }
    wait "$CH_PID" 2>/dev/null
  fi
  if [[ -n "$LISTENER_PID" ]] && kill -0 "$LISTENER_PID" 2>/dev/null; then
    echo "listener pid $LISTENER_PID still running -> SIGTERM"
    kill -TERM "$LISTENER_PID"; wait "$LISTENER_PID" 2>/dev/null
  fi
  if ip link show "$TAP" >/dev/null 2>&1; then
    echo "tap $TAP still present after CH exit -> deleting"
    ip link del "$TAP"
  fi
  [[ -d "$RUN_DIR" && "$RUN_DIR" == "$SCRATCH"/runs/* ]] && rm -rf -- "$RUN_DIR"
  rmdir "$SCRATCH/runs" 2>/dev/null
  section "post-run host state (must match pre-run)"
  AFTER_LINKS="$(snap_links)"; AFTER_CH="$(snap_ch)"
  echo "links added:   $(comm -13 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "links removed: $(comm -23 <(echo "$BEFORE_LINKS") <(echo "$AFTER_LINKS") | tr '\n' ' ')"
  echo "cloud-hypervisor processes before: [${BEFORE_CH}] after: [${AFTER_CH}]"
  echo "tap $TAP present: $(ip link show "$TAP" >/dev/null 2>&1 && echo yes || echo no)"
  echo "run dir present: $([[ -e "$RUN_DIR" ]] && echo yes || echo no)"
  echo "leftover listener/CH pids: $(pgrep -f "$RUN_DIR" | tr '\n' ' ')"
}

section "pre-run host state"
BEFORE_LINKS="$(snap_links)"
BEFORE_CH="$(snap_ch)"
echo "links: $(echo "$BEFORE_LINKS" | tr '\n' ' ')"
echo "cloud-hypervisor processes: [${BEFORE_CH}]"
if ip link show "$TAP" >/dev/null 2>&1; then echo "FATAL: tap $TAP already exists"; exit 2; fi
ip -4 addr show | grep -q "192\.168\.203\." && { echo "FATAL: 192.168.203.0/24 already in use"; exit 2; }
trap cleanup EXIT HUP INT TERM

section "versions / artifacts"
uname -r
cloud-hypervisor --version | head -1
git -C "$SCRATCH/nanos" rev-parse HEAD
sha256sum "$KERNEL" "$IMAGE"

mkdir -p "$RUN_DIR"
cp --reflink=auto "$IMAGE" "$RUN_DIR/disk.img"   # Nanos writes klog/TFS log; keep the built image pristine
readonly VSOCK_BASE="$RUN_DIR/vsock.sock"

section "launch"
T0="$(date +%s.%N)"
python3 "$INCREMENT/host_listener.py" "${VSOCK_BASE}_${VSOCK_PORT}" "$TAP_IP" "$NET_PORT" "$T0" "$BOOT_TIMEOUT_S" \
  > "$RUN_DIR/listener.log" 2>&1 &
LISTENER_PID=$!
for _ in $(seq 1 50); do [[ -S "${VSOCK_BASE}_${VSOCK_PORT}" ]] && break; sleep 0.1; done

CH_CMD=(cloud-hypervisor
  --kernel "$KERNEL"
  --disk "path=$RUN_DIR/disk.img"
  --cpus boot=1
  --memory size=512M
  --net "tap=$TAP,ip=$TAP_IP,mask=255.255.255.0,mac=52:54:00:1a:03:02"
  --vsock "cid=$GUEST_CID,socket=$VSOCK_BASE"
  --serial "file=$RUN_DIR/serial.log"
  --console off
  --event-monitor "path=$RUN_DIR/events.json"
  --log-file "$RUN_DIR/ch.log"
  -vv)
printf 'CH command: '; printf '%q ' "${CH_CMD[@]}"; echo
"${CH_CMD[@]}" > "$RUN_DIR/ch.stdout" 2>&1 &
CH_PID=$!
echo "cloud-hypervisor pid=$CH_PID listener pid=$LISTENER_PID T0=$T0"

# While CH runs: wait for the tap, then show it once (proves CH created it).
for _ in $(seq 1 50); do ip link show "$TAP" >/dev/null 2>&1 && break; sleep 0.1; done
ip -br addr show "$TAP" 2>&1 | sed 's/^/tap while VM running: /'

CH_EXIT=""
for _ in $(seq 1 $((BOOT_TIMEOUT_S * 10))); do
  if ! kill -0 "$CH_PID" 2>/dev/null; then wait "$CH_PID"; CH_EXIT=$?; break; fi
  sleep 0.1
done
T_END="$(date +%s.%N)"
if [[ -n "$CH_EXIT" ]]; then
  echo "cloud-hypervisor exited on its own: exit=$CH_EXIT after $(python3 -c "print(f'{$T_END-$T0:.3f}')")s"
else
  echo "cloud-hypervisor still running after ${BOOT_TIMEOUT_S}s (guest did not power off) -> cleanup will stop it"
fi
wait "$LISTENER_PID"; LISTENER_EXIT=$?; LISTENER_PID=""
echo "listener exit=$LISTENER_EXIT"

section "host listener log"
cat "$RUN_DIR/listener.log"
section "guest serial console (Nanos)"
cat "$RUN_DIR/serial.log" 2>&1
section "cloud-hypervisor event monitor"
cat "$RUN_DIR/events.json" 2>&1
section "cloud-hypervisor stdout/stderr"
cat "$RUN_DIR/ch.stdout" 2>&1
section "cloud-hypervisor log (-vv)"
cat "$RUN_DIR/ch.log" 2>&1

section "run verdict"
grep -h "VERDICT" "$RUN_DIR/listener.log" "$RUN_DIR/serial.log" 2>/dev/null
exit "$LISTENER_EXIT"
