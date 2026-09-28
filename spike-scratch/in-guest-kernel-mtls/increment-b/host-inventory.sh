#!/usr/bin/env bash
# Read-only host inventory for Spike A2 (Firecracker): substrate probes
# (testing.md, fail closed), Firecracker presence, and the Spike A artifacts this
# increment reuses. Creates nothing.
set -uo pipefail

section() { printf '\n=== %s ===\n' "$*"; }
fail=0
SCRATCH="$HOME/igkm-spike-a"

section "substrate probes"
printf 'uname -m: %s\n' "$(uname -m)"
[[ "$(uname -m)" == x86_64 ]] || { echo "FAIL: not x86_64"; fail=1; }
printf 'uname -r: %s\n' "$(uname -r)"
virt="$(systemd-detect-virt 2>&1)"; vstat=$?
printf 'systemd-detect-virt: %s (exit %s)\n' "$virt" "$vstat"
[[ "$virt" == none ]] || { echo "FAIL: virtualized host"; fail=1; }
printf 'cpu vmx/svm flag count: %s\n' "$(grep -cE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo)"
grep -qE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo || { echo "FAIL: no vmx/svm"; fail=1; }
ls -l /dev/kvm
[[ -c /dev/kvm ]] || { echo "FAIL: /dev/kvm is not a character device"; fail=1; }
python3 - <<'PY' || fail=1
import fcntl, os
fd = os.open("/dev/kvm", os.O_RDWR | os.O_CLOEXEC)
v = fcntl.ioctl(fd, 0xAE00, 0)
vm = fcntl.ioctl(fd, 0xAE01, 0)
os.close(vm); os.close(fd)
print(f"KVM_GET_API_VERSION={v} KVM_CREATE_VM=ok")
assert v == 12
PY
grep -m1 'model name' /proc/cpuinfo
nproc

section "firecracker on the host"
printf 'firecracker on PATH: %s\n' "$(command -v firecracker 2>/dev/null || echo MISSING)"
printf 'jailer on PATH:      %s\n' "$(command -v jailer 2>/dev/null || echo MISSING)"
ls -la "$SCRATCH/firecracker" 2>&1 | head -5
ls -l /dev/net/tun

section "identity / scratch"
id
echo "HOME=$HOME"
df -h "$HOME" | tail -1

section "Spike A artifacts reused (must exist, unchanged)"
git -C "$SCRATCH/nanos" rev-parse HEAD
echo "stock tree dirty files: $(git -C "$SCRATCH/nanos" status --porcelain | wc -l)"
sha256sum "$SCRATCH/nanos/output/platform/pc/bin/kernel.img" \
  "$SCRATCH/out/kernel-pvhfix.img" \
  "$SCRATCH/out/nanos-vsock.img" \
  "$SCRATCH/out/vsock_probe" 2>&1
readelf -n "$SCRATCH/nanos/output/platform/pc/bin/kernel.img" | grep -A2 Xen
readelf -h "$SCRATCH/nanos/output/platform/pc/bin/kernel.img" | grep 'Entry point'

section "network reachability (HEAD/metadata only)"
printf 'github.com -> %s\n' "$(curl -sS -o /dev/null -m 15 -w '%{http_code}' https://github.com 2>&1)"

section "tap name / subnet availability"
echo "igkmb0 present: $(ip link show igkmb0 >/dev/null 2>&1 && echo yes || echo no)"
echo "192.168.203.x on host: $(ip -4 -br addr | grep -c '192\.168\.203\.')"
ip -4 -br addr
ip -4 route

section "staging files for preflight"
ls -l /srv/vm/overdrive-testing/ 2>&1 | head

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
