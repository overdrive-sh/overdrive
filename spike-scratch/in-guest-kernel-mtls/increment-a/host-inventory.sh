#!/usr/bin/env bash
# Read-only host inventory: substrate probes (testing.md, fail closed) plus the
# userspace tooling this probe needs. Creates nothing.
set -uo pipefail

section() { printf '\n=== %s ===\n' "$*"; }
fail=0

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
free -g | head -2

section "cloud hypervisor"
command -v cloud-hypervisor
cloud-hypervisor --version

section "identity / scratch"
id
echo "HOME=$HOME"
df -h "$HOME" | tail -1

section "build tooling"
for t in gcc cc make nasm ld objcopy strip git curl wget python3 socat qemu-img tar xz rustc cargo go; do
  printf '%-10s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
gcc --version 2>/dev/null | head -1
ls /usr/lib/x86_64-linux-gnu/libc.a 2>/dev/null || echo "libc.a MISSING"
ls /usr/include/linux/vm_sockets.h 2>/dev/null || echo "linux/vm_sockets.h MISSING"
rustup target list --installed 2>/dev/null || echo "rustup unavailable"

section "network reachability (HEAD requests only)"
for u in https://github.com https://storage.googleapis.com/nanos/release/latest.txt https://www.nasm.us; do
  printf '%s -> %s\n' "$u" "$(curl -sS -o /dev/null -m 15 -w '%{http_code}' "$u" 2>&1)"
done
curl -sS -m 15 https://storage.googleapis.com/nanos/release/latest.txt; echo

section "host routes (to pick a non-colliding tap subnet)"
ip -4 -br addr
ip -4 route

section "staging files for preflight"
ls -l /srv/vm/overdrive-testing/ 2>&1 | head

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
