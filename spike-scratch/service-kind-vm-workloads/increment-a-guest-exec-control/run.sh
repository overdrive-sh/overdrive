#!/usr/bin/env bash
set -euo pipefail

readonly STAGING=/srv/vm/overdrive-testing
readonly INCREMENT=spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control
readonly BUILD="$INCREMENT/target/x86_64-unknown-linux-musl/release"
ROOT="$(mktemp -d "$STAGING/service-vm-h1-h5.XXXXXX")"
readonly ROOT
readonly MARKER="$ROOT/.service-vm-spike-owned"
touch "$MARKER"
MNT="$ROOT/mnt"
LOOP=""

owned_vmm_pids() {
  for proc in /proc/[0-9]*; do
    [[ -r "$proc/cmdline" ]] || continue
    local cmd
    cmd="$(tr '\0' ' ' < "$proc/cmdline" 2>/dev/null || true)"
    [[ "$cmd" == *"$ROOT"* && "$cmd" == *cloud-hypervisor* ]] && basename "$proc"
  done
}

cleanup() {
  set +e
  for pid in $(owned_vmm_pids); do kill -TERM "$pid" 2>/dev/null; done
  for _ in $(seq 1 30); do [[ -z "$(owned_vmm_pids)" ]] && break; sleep 0.1; done
  for pid in $(owned_vmm_pids); do kill -KILL "$pid" 2>/dev/null; done
  mountpoint -q "$MNT" && umount "$MNT"
  [[ -n "$LOOP" ]] && losetup -d "$LOOP" 2>/dev/null
  [[ -f "$MARKER" ]] && rm -rf -- "$ROOT"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$MNT"
cp --reflink=always "$STAGING/rootfs.ext4" "$ROOT/rootfs.ext4"
LOOP="$(losetup --find --show "$ROOT/rootfs.ext4")"
mount "$LOOP" "$MNT"
install -m 0755 "$BUILD/spike-init" "$MNT/sbin/init"
install -m 0755 "$BUILD/spike-init" "$MNT/init"
install -m 0755 "$BUILD/spike-probe" "$MNT/sbin/spike-probe"
install -m 0755 "$BUILD/spike-workload" "$MNT/sbin/spike-workload"
sync "$MNT"
umount "$MNT"
losetup -d "$LOOP"
LOOP=""

echo "SUBSTRATE cloud_hypervisor=$(cloud-hypervisor --version | head -n 1) kernel=$(uname -sr) arch=$(uname -m) virtualization=$(systemd-detect-virt || true)"
timeout 180s python3 "$INCREMENT/host_controller.py" \
  "$ROOT" "$STAGING/kernel" "$ROOT/rootfs.ext4"

[[ -z "$(owned_vmm_pids)" ]]
echo "H1_H5_COMPLETE owned_vmm_residual=0"
