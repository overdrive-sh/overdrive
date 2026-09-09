#!/usr/bin/env bash
set -euo pipefail

readonly STAGING=/srv/vm/overdrive-testing
readonly KERNEL="$STAGING/kernel"
readonly MASTER="$STAGING/rootfs.ext4"
readonly INCREMENT=spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe
readonly GUEST_BIN="$INCREMENT/target/x86_64-unknown-linux-musl/release/spike-workload"
readonly WORKLOAD_ID=svm-h6
readonly GUEST_PORT=18443
ROOT="$(mktemp -d "$STAGING/service-vm-h6.XXXXXX")"
readonly ROOT
readonly MARKER="$ROOT/.service-vm-spike-owned"
touch "$MARKER"
CFG="$(mktemp -d /tmp/service-vm-h6-cfg.XXXXXX)"
CREDS="$(mktemp -d /tmp/service-vm-h6-creds.XXXXXX)"
DATA="$(mktemp -d "$STAGING/service-vm-h6-data.XXXXXX")"
MNT="$ROOT/mnt"
LOOP=""
SERVE_PID=""
STARTED=0

snapshot_netns() { ip netns list | awk '{print $1}' | sort; }
snapshot_links() { ip -o link show | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | sort; }
snapshot_scopes() { find /sys/fs/cgroup/overdrive.slice/workloads.slice -maxdepth 1 -type d -name 'alloc-*.scope' -printf '%f\n' 2>/dev/null | sort; }
snapshot_runs() { find /run/overdrive/vm -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort; }
owned_ch() {
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    local cmd
    cmd="$(tr '\0' ' ' < "$proc/cmdline" 2>/dev/null || true)"
    [[ "$cmd" == *"$ROOT/rootfs.ext4"* && "$cmd" == *cloud-hypervisor* ]] && basename "$proc"
  done
}
new_only() { comm -13 <(printf '%s\n' "$1" | sed '/^$/d') <(printf '%s\n' "$2" | sed '/^$/d'); }

BEFORE_NETNS="$(snapshot_netns)"
BEFORE_LINKS="$(snapshot_links)"
BEFORE_SCOPES="$(snapshot_scopes)"
BEFORE_RUNS="$(snapshot_runs)"

cleanup() {
  set +e
  if [[ "$STARTED" -eq 1 && -n "$SERVE_PID" ]] && kill -0 "$SERVE_PID" 2>/dev/null; then
    OVERDRIVE_CONFIG_DIR="$CFG" target/debug/overdrive job stop "$WORKLOAD_ID" >/dev/null 2>&1
  fi
  [[ -n "$SERVE_PID" ]] && kill "$SERVE_PID" 2>/dev/null
  [[ -n "$SERVE_PID" ]] && wait "$SERVE_PID" 2>/dev/null
  for pid in $(owned_ch); do kill -TERM "$pid" 2>/dev/null; done
  for _ in $(seq 1 30); do [[ -z "$(owned_ch)" ]] && break; sleep 0.1; done
  for pid in $(owned_ch); do kill -KILL "$pid" 2>/dev/null; done
  for scope in $(new_only "$BEFORE_SCOPES" "$(snapshot_scopes)"); do
    echo 1 > "/sys/fs/cgroup/overdrive.slice/workloads.slice/$scope/cgroup.kill" 2>/dev/null
    rmdir "/sys/fs/cgroup/overdrive.slice/workloads.slice/$scope" 2>/dev/null
  done
  for netns in $(new_only "$BEFORE_NETNS" "$(snapshot_netns)"); do
    [[ "$netns" == ovd-ns-* ]] && ip netns del "$netns" 2>/dev/null
  done
  for link in $(new_only "$BEFORE_LINKS" "$(snapshot_links)"); do
    [[ "$link" == ovd-* ]] && ip link del "$link" 2>/dev/null
  done
  mountpoint -q "$MNT" && umount "$MNT"
  [[ -n "$LOOP" ]] && losetup -d "$LOOP" 2>/dev/null
  if command -v keyctl >/dev/null 2>&1; then keyctl purge user overdrive:ca:kek:overdrive-ca-root >/dev/null 2>&1; fi
  [[ -f "$MARKER" ]] && rm -rf -- "$ROOT"
  [[ "$CFG" == /tmp/service-vm-h6-cfg.* ]] && rm -rf -- "$CFG"
  [[ "$CREDS" == /tmp/service-vm-h6-creds.* ]] && rm -rf -- "$CREDS"
  [[ "$DATA" == "$STAGING"/service-vm-h6-data.* ]] && rm -rf -- "$DATA"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$MNT"
cp --reflink=always "$MASTER" "$ROOT/rootfs.ext4"
LOOP="$(losetup --find --show "$ROOT/rootfs.ext4")"
mount "$LOOP" "$MNT"
install -m 0755 "$GUEST_BIN" "$MNT/sbin/spike-workload"
sync "$MNT"
umount "$MNT"
losetup -d "$LOOP"
LOOP=""

cat > "$ROOT/spec.toml" <<EOF
[job]
id = "$WORKLOAD_ID"

[vm]
command = "/sbin/spike-workload"
args = ["listener", "$GUEST_PORT"]
kernel = "$KERNEL"
rootfs = "$ROOT/rootfs.ext4"

[resources]
cpu_milli = 500
memory_bytes = 134217728
EOF

OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build >/dev/null
cargo build -p overdrive-cli --bin overdrive >/dev/null
chmod 0711 "$DATA"
chmod 0700 "$CREDS"
head -c 32 /dev/urandom > "$CREDS/overdrive-ca-root"
chmod 0400 "$CREDS/overdrive-ca-root"
BIND_PORT="$(python3 - <<'PY'
import socket
s=socket.socket(); s.bind(('127.0.0.1',0)); print(s.getsockname()[1]); s.close()
PY
)"
OVERDRIVE_CONFIG_DIR="$CFG" CREDENTIALS_DIRECTORY="$CREDS" \
  target/debug/overdrive serve --bind "127.0.0.1:$BIND_PORT" --data-dir "$DATA" \
  > "$ROOT/serve.log" 2>&1 &
SERVE_PID=$!
for _ in $(seq 1 120); do
  [[ -f "$CFG/.overdrive/config" ]] && break
  kill -0 "$SERVE_PID" 2>/dev/null || { tail -80 "$ROOT/serve.log"; exit 1; }
  sleep 0.25
done
[[ -f "$CFG/.overdrive/config" ]]
STARTED=1

OVERDRIVE_CONFIG_DIR="$CFG" target/debug/overdrive deploy --detach "$ROOT/spec.toml" >/dev/null
STATE=""
for _ in $(seq 1 120); do
  OVERDRIVE_CONFIG_DIR="$CFG" target/debug/overdrive workload describe "$WORKLOAD_ID" > "$ROOT/describe.out" 2>&1 || true
  STATE="$(awk 'index($0,"Attempt")==1 && $2=="State" {table=1; next} table && $1~/^[0-9]+$/ {print $2; exit}' "$ROOT/describe.out")"
  [[ "$STATE" == Running ]] && break
  sleep 0.5
done
[[ "$STATE" == Running ]] || { cat "$ROOT/describe.out"; tail -80 "$ROOT/serve.log"; exit 1; }

GUEST_ADDR="$(awk '/^Addresses:/{addresses=1; next} addresses && $1 ~ /^alloc-/ {print $2; exit}' "$ROOT/describe.out")"
[[ "$GUEST_ADDR" =~ ^10\.99\.[0-9]+\.[0-9]+$ ]]
NEW_NETNS="$(new_only "$BEFORE_NETNS" "$(snapshot_netns)")"
[[ "$(printf '%s\n' "$NEW_NETNS" | grep -c '^ovd-ns-')" -eq 1 ]]
NETNS="$(printf '%s\n' "$NEW_NETNS" | grep '^ovd-ns-')"
TAP="$(ip netns exec "$NETNS" ip -o link show | sed -nE 's/^[0-9]+: (ovd-tp-[^:@]+).*/\1/p' | head -n 1)"
[[ -n "$TAP" ]]
HOST_VETH="$(ip route get "$GUEST_ADDR" | awk '{for(i=1;i<=NF;i++) if($i=="dev") {print $(i+1); exit}}')"
[[ "$HOST_VETH" == ovd-hv-* ]]
ip netns exec "$NETNS" ip -4 addr show dev "$TAP" > "$ROOT/tap-address.txt"
ip route get "$GUEST_ADDR" > "$ROOT/host-route.txt"

TCPDUMP_PID=""
if command -v tcpdump >/dev/null 2>&1; then
  timeout 10s ip netns exec "$NETNS" tcpdump -nn -i "$TAP" -c 2 "tcp port $GUEST_PORT" > "$ROOT/tcpdump.txt" 2>&1 &
  TCPDUMP_PID=$!
  sleep 0.3
fi
RESPONSE="$(python3 - "$GUEST_ADDR" "$GUEST_PORT" <<'PY'
import socket,sys
s=socket.create_connection((sys.argv[1],int(sys.argv[2])),timeout=5)
s.sendall(b'H6-HOST-PROBE\n')
print(s.recv(64).decode().strip())
s.close()
PY
)"
[[ "$RESPONSE" == H6-GUEST-OK ]]
if [[ -n "$TCPDUMP_PID" ]]; then wait "$TCPDUMP_PID"; fi
[[ ! -f "$ROOT/tcpdump.txt" ]] || grep -q "Flags \[S\]" "$ROOT/tcpdump.txt"

echo "SUBSTRATE cloud_hypervisor=$(cloud-hypervisor --version | head -n 1) kernel=$(uname -sr) arch=$(uname -m) virtualization=$(systemd-detect-virt || true)"
echo "H6 PASS source_namespace=host target=guest_workload_addr route_dev=$HOST_VETH netns=$NETNS tap=$TAP response=$RESPONSE"
if [[ -f "$ROOT/tcpdump.txt" ]]; then echo "H6_WIRE PASS tap_capture_syn=observed"; fi

OVERDRIVE_CONFIG_DIR="$CFG" target/debug/overdrive job stop "$WORKLOAD_ID" >/dev/null
for _ in $(seq 1 60); do
  OVERDRIVE_CONFIG_DIR="$CFG" target/debug/overdrive workload describe "$WORKLOAD_ID" > "$ROOT/after-stop.out" 2>&1 || true
  STOP_STATE="$(awk 'index($0,"Attempt")==1 && $2=="State" {table=1; next} table && $1~/^[0-9]+$/ {print $2; exit}' "$ROOT/after-stop.out")"
  [[ "$STOP_STATE" == Terminated ]] && break
  sleep 0.5
done
[[ "$STOP_STATE" == Terminated ]]
STARTED=0
kill "$SERVE_PID"
wait "$SERVE_PID" || true
SERVE_PID=""
for _ in $(seq 1 60); do
  [[ -z "$(owned_ch)" ]] && [[ -z "$(new_only "$BEFORE_NETNS" "$(snapshot_netns)")" ]] && break
  sleep 0.25
done
[[ -z "$(owned_ch)" ]]
[[ -z "$(new_only "$BEFORE_NETNS" "$(snapshot_netns)")" ]]
[[ -z "$(new_only "$BEFORE_SCOPES" "$(snapshot_scopes)")" ]]
[[ -z "$(new_only "$BEFORE_RUNS" "$(snapshot_runs)")" ]]
echo "H6_CLEANUP PASS owned_vmm=0 netns_delta=0 scope_delta=0 run_dir_delta=0"
