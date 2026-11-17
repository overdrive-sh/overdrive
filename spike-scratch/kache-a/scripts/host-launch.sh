#!/usr/bin/env bash
# Host half of the kache spike. Runs as root inside the native_launcher private
# network/mount/cgroup namespaces. It only prepares boot artifacts and launches
# Cloud Hypervisor guests; every build, cache and Garage process runs inside a
# guest. Usage: host-launch.sh <plan>   (plans/<plan>.sh)
set -euo pipefail

PLAN_LINE="$1"
read -r PLAN PLAN_ARGS <<< "$PLAN_LINE"
read -ra PA <<< "${PLAN_ARGS:-}"
SPIKE="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$(cd "$SPIKE/../.." && pwd)"
STATE="$SPIKE/target/vmstate"          # persistent across runs; never synced
RUN="$(date -u +%Y%m%dT%H%M%SZ)-$PLAN${PA[0]:+-${PA[0]}}"
EV="$SPIKE/evidence/$RUN"              # pulled back with `cargo xtask metal pull`
BUS="$STATE/bus/$RUN"                  # read-write share every guest sees
SOCK="$(mktemp -d /tmp/ks.XXXXXX)"      # unix sockets: path must be < SUN_LEN (108)
GARAGE_SHA256=2ff667286217cd73ebcb1c03af993822365000441dfe31a80cb64c5189a9dea8
KACHE_TARBALL="$SPIKE/staging/kache-2.1.1-vendored.tar.zst"
MIN_FREE_GB=25      # hard floor on the shared box; a plan aborts below it
VM_BUDGET_GB=22    # a full-workspace dev VM adds ~19 GB (target dir + wrapper store); wait for this much room

mkdir -p "$EV" "$BUS" "$STATE/disks"
log() { printf '%s host %s\n' "$(date -u +%FT%T.%3NZ)" "$*" | tee -a "$EV/host.log" >&2; }

declare -a PIDS=()
cleanup() {
  local rc=$?
  touch "$BUS/stop" 2>/dev/null || true
  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  sleep 2
  for pid in "${PIDS[@]}"; do kill -9 "$pid" 2>/dev/null || true; done
  # Keep evidence: everything guests wrote to the bus except bulky stores.
  rsync -a --exclude='*.store/' "$BUS/" "$EV/bus/" 2>/dev/null || cp -a "$BUS/." "$EV/bus/" || true
  rm -rf "$SOCK"
  chown -R --reference="$SPIKE" "$EV" "$BUS" 2>/dev/null || true   # pull runs as the login user
  log "evidence at $EV: $(find "$EV" -type f | wc -l) files, owner $(stat -c %U "$EV"), mount $(findmnt -no TARGET,FSTYPE -T "$EV" | tr "\n" " ")"
  log "exit rc=$rc"
  # Another session syncs this checkout with --delete, which can remove
  # $EV before it is pulled; carry the evidence in the run output as well
  # (decode with extract-evidence.sh) and archive it under target/ (never synced).
  [ "$PLAN" = cleanup ] || { mkdir -p "$STATE/evidence-archive"; cp -a "$EV" "$STATE/evidence-archive/" 2>/dev/null || true; }
  { echo "=====EVIDENCE-BEGIN $RUN"; tar -C "$EV" -czf - . | base64; echo "=====EVIDENCE-END $RUN"; } >&2
  exit "$rc"
}
trap cleanup EXIT

free_gb() { df -BG --output=avail "$STATE" | tail -1 | tr -dc 0-9; }

# ---- host facts ------------------------------------------------------------
{
  echo "run=$RUN plan=$PLAN"
  echo "metal_run_id=${OVERDRIVE_METAL_RUN_ID:-}"
  echo "uname=$(uname -r)"
  echo "nproc=$(nproc)"
  free -g | sed -n 2p | awk '{print "mem_total_gb="$2}'
  df -hT "$STATE" | tail -1 | awk '{print "fs="$2" size="$3" avail="$5}'
  [ -f "$SRC/.overdrive-metal-source" ] && sed 's/^/source: /' "$SRC/.overdrive-metal-source"
  echo "cloud_hypervisor=$(cloud-hypervisor --version | head -1)"
  echo "virtiofsd=$(/usr/libexec/virtiofsd --version 2>&1 | head -1)"
} > "$EV/host-facts.txt"
cat "$EV/host-facts.txt" >&2
[ "$(free_gb)" -ge "$MIN_FREE_GB" ] || { log "FATAL free disk $(free_gb)G < ${MIN_FREE_GB}G"; exit 1; }
/usr/libexec/virtiofsd --help 2>&1 | grep -q -- '--readonly' \
  || { log "FATAL virtiofsd has no --readonly; refusing to share / writable"; exit 1; }

if [ "$PLAN" != cleanup ]; then
# ---- boot artifacts (cached in $STATE) --------------------------------------
KERNEL="/boot/vmlinuz-$(uname -r)"
INITRD="$STATE/initramfs-$(uname -r).cpio.gz"
if [ ! -f "$INITRD" ]; then
  log "building spike-init + initramfs"
  ( cd "$SPIKE" && cargo build --release --offline --target x86_64-unknown-linux-musl --bin spike-init )
  ir="$(mktemp -d)"
  mkdir -p "$ir/dev" "$ir/proc" "$ir/newroot"
  cp "$SPIKE/target/x86_64-unknown-linux-musl/release/spike-init" "$ir/init"
  ko="$(find "/lib/modules/$(uname -r)" -name 'virtiofs.ko*' | head -1)"
  case "$ko" in
    *.zst) zstd -dq "$ko" -o "$ir/virtiofs.ko" ;;
    *.xz) xz -dc "$ko" > "$ir/virtiofs.ko" ;;
    *) cp "$ko" "$ir/virtiofs.ko" ;;
  esac
  ( cd "$ir" && find . | cpio -o -H newc --quiet | gzip -1 ) > "$INITRD.tmp"
  mv "$INITRD.tmp" "$INITRD"
  rm -rf "$ir"
fi

KACHE_ROOT="$STATE/kache"
if [ ! -x "$KACHE_ROOT/bin/kache" ]; then
  log "building kache 2.1.1 offline from vendored sources"
  rm -rf "$STATE/kache-src"; mkdir -p "$STATE/kache-src"
  zstd -dc "$KACHE_TARBALL" | tar x --warning=no-unknown-keyword -C "$STATE/kache-src"
  printf '\n[workspace]\n' >> "$STATE/kache-src/kache-2.1.1/Cargo.toml"
  ( cd "$STATE/kache-src/kache-2.1.1" && \
    RUSTUP_TOOLCHAIN=1.95.0 cargo install --path . --locked --offline --root "$KACHE_ROOT" ) \
    > "$EV/kache-build.log" 2>&1 || { log "FATAL kache build failed"; tail -40 "$EV/kache-build.log" >&2; exit 1; }
fi
sha256sum "$KACHE_ROOT"/bin/* > "$EV/kache-binaries.sha256"
"$KACHE_ROOT/bin/kache" --version > "$EV/kache-version.txt" 2>&1 || true
echo "$GARAGE_SHA256  $SPIKE/staging/garage" | sha256sum -c - >&2 \
  || { log "FATAL garage checksum mismatch"; exit 1; }

fi

# ---- network: one bridge in the private netns -------------------------------
ip link set lo up
ip link add br0 type bridge 2>/dev/null || true
ip addr add 10.77.0.1/24 dev br0 2>/dev/null || true
ip link set br0 up

# ---- VM launch ---------------------------------------------------------------
declare -A VM_PID=() VM_FS=()
# boot_vm NAME IP VCPUS MEM_GB DISK_GB ROLE [KEY=VALUE ...]
boot_vm() {
  local name="$1" ip="$2" vcpus="$3" mem="$4" disk_gb="$5" role="$6"; shift 6
  local disk="$STATE/disks/$name.img" tap="tap-$name"
  if [ ! -f "$disk" ]; then
    truncate -s "${disk_gb}G" "$disk"
    mkfs.ext4 -q -F -L "$name" "$disk"
  fi
  ip tuntap add "$tap" mode tap 2>/dev/null || true
  ip link set "$tap" master br0 up
  /usr/libexec/virtiofsd --socket-path="$SOCK/$name-root.sock" --shared-dir=/ \
    --readonly --cache=auto --sandbox=none --announce-submounts \
    > "$EV/$name.virtiofsd-root.log" 2>&1 &
  PIDS+=("$!"); VM_FS[$name]="$!"
  /usr/libexec/virtiofsd --socket-path="$SOCK/$name-bus.sock" --shared-dir="$BUS" \
    --cache=never --sandbox=none > "$EV/$name.virtiofsd-bus.log" 2>&1 &
  PIDS+=("$!"); VM_FS[$name]+=" $!"
  for _ in $(seq 50); do
    [ -S "$SOCK/$name-root.sock" ] && [ -S "$SOCK/$name-bus.sock" ] && break; sleep 0.1
  done
  local mac; mac="$(printf '52:54:00:%02x:%02x:%02x' $((RANDOM%256)) $((RANDOM%256)) $((RANDOM%256)))"
  local cmdline="console=ttyS0 reboot=k panic=1 SPIKE_INIT=$SPIKE/scripts/guest-init.sh"
  cmdline+=" SPIKE_ROLE=$role SPIKE_NAME=$name SPIKE_IP=$ip SPIKE_SRC=$SRC SPIKE_DIR=$SPIKE"
  cmdline+=" SPIKE_RUN=$RUN SPIKE_VCPUS=$vcpus"
  local kv; for kv in "$@"; do cmdline+=" $kv"; done
  log "boot $name ip=$ip vcpus=$vcpus mem=${mem}G disk=${disk_gb}G role=$role $*"
  cloud-hypervisor --kernel "$KERNEL" --initramfs "$INITRD" --cmdline "$cmdline" \
    --cpus "boot=$vcpus" --memory "size=${mem}G,shared=on" \
    --fs "tag=rootfs,socket=$SOCK/$name-root.sock" --fs "tag=bus,socket=$SOCK/$name-bus.sock" \
    --disk "path=$disk,image_type=raw" --net "tap=$tap,mac=$mac" \
    --serial "file=$EV/$name.console" --console off \
    > "$EV/$name.ch.log" 2>&1 &
  VM_PID[$name]=$!
  PIDS+=("$!")
  [ "$role" = cache ] || echo "$!" >> "$SOCK/vm.pids"
}

# wait_file PATH TIMEOUT_S
wait_file() {
  local path="$1" timeout="$2" t=0
  while [ ! -e "$path" ]; do
    sleep 2; t=$((t + 2))
    for p in "${VM_PID[@]:-}"; do [ -z "$p" ] || kill -0 "$p" 2>/dev/null || { log "a guest VMM exited while waiting for $path"; return 1; }; done
    [ "$t" -lt "$timeout" ] || { log "timeout waiting for $path"; return 1; }
  done
}

# wait_vms NAME... : wait for the VMs' CH processes; fail if any guest's exit != 0
wait_vms() {
  local name rc=0
  for name in "$@"; do
    wait "${VM_PID[$name]}" || true
    local code; code="$(cat "$BUS/$name/exit" 2>/dev/null || echo missing)"
    log "vm $name finished exit=$code"
    [ "$code" = 0 ] || rc=1
  done
  return "$rc"
}

# run_vm NAME IP VCPUS MEM_GB DISK_GB ROLE [KEY=VALUE ...]: fresh disk, boot, wait, drop the disk.
run_vm() {
  local name="$1"; ensure_room; rm -f "$STATE/disks/$name.img"
  boot_vm "$@"
  local rc=0; wait_vms "$name" || rc=1
  # shellcheck disable=SC2086
  kill ${VM_FS[$name]} 2>/dev/null || true
  rm -f "$STATE/disks/$name.img"
  [ ! -e "$BUS/abort" ] || { log "plan aborted by the disk guard"; exit 3; }
  return "$rc"
}

disk_guard() {
  while sleep 20; do
    if [ "$(free_gb)" -lt "$MIN_FREE_GB" ]; then
      log "FATAL free disk $(free_gb)G below ${MIN_FREE_GB}G; aborting plan, stopping dev guests"
      touch "$BUS/abort"
      # shellcheck disable=SC2046
      [ -f "$SOCK/vm.pids" ] && kill $(cat "$SOCK/vm.pids") 2>/dev/null || true
      return
    fi
  done
}
# ensure_room_n N: a wave of N dev VMs needs N budgets.
ensure_room_n() { local t=0 b="${2:-$VM_BUDGET_GB}"; while [ "$(free_gb)" -lt "$((MIN_FREE_GB + b * $1))" ]; do
  [ "$t" -lt 1800 ] || { log "FATAL no room for $1 VMs"; exit 3; }; sleep 60; t=$((t + 60)); done; }
# ensure_room: wait (up to 30 min) until a dev VM fits, abort the plan otherwise.
ensure_room() {
  local t=0
  [ ! -e "$BUS/abort" ] || { log "plan aborted earlier"; exit 3; }
  while [ "$(free_gb)" -lt "$((MIN_FREE_GB + VM_BUDGET_GB))" ]; do
    [ "$t" -lt 1800 ] || { log "FATAL no room: free $(free_gb)G after 30 min"; exit 3; }
    [ "$t" -gt 0 ] || log "waiting for disk room: free $(free_gb)G, need $((MIN_FREE_GB + VM_BUDGET_GB))G"
    sleep 60; t=$((t + 60))
  done
}
disk_guard & PIDS+=("$!")
( while sleep 30; do echo "$(date -u +%T) $(cat /proc/loadavg) free_gb=$(free_gb)"; done >> "$EV/host-load.log" ) & PIDS+=("$!")

# shellcheck source=/dev/null
source "$SPIKE/plans/$PLAN.sh"
plan_main
