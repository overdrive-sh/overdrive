#!/usr/bin/env bash
# Read-only host inventory for Spike B3 (GH #303, increment-f): substrate
# probes (testing.md, fail closed), the Firecracker binary, the Unikraft /
# lib-lwip / Mbed TLS trees Spikes A3/B/B2 left in ~/igkm-spike-a, the host
# C toolchain and glibc that build the UNMODIFIED Linux test binary
# (static-pie support), fetch tools, and port/subnet availability.
# Creates nothing.
set -uo pipefail

section() { printf '\n=== %s ===\n' "$*"; }
fail=0
SCRATCH="$HOME/igkm-spike-a"
W="$SCRATCH/unikraft"

section "substrate probes"
printf 'uname -m: %s\n' "$(uname -m)"
[[ "$(uname -m)" == x86_64 ]] || { echo "FAIL: not x86_64"; fail=1; }
printf 'uname -r: %s\n' "$(uname -r)"
virt="$(systemd-detect-virt 2>&1)"; vstat=$?
printf 'systemd-detect-virt: %s (exit %s)\n' "$virt" "$vstat"
[[ "$virt" == none ]] || { echo "FAIL: virtualized host"; fail=1; }
printf 'cpu vmx/svm flag count: %s\n' "$(grep -cE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo)"
grep -qE '^flags.*\b(vmx|svm)\b' /proc/cpuinfo || { echo "FAIL: no vmx/svm"; fail=1; }
printf 'cpu aes flag count: %s\n' "$(grep -cE '^flags.*\baes\b' /proc/cpuinfo)"
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

section "Firecracker v1.17.0 (Spike A2 binary, must be unchanged)"
bin="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
sha256sum "$bin" 2>&1
"$bin" --version 2>&1 | head -1

section "Unikraft / lib-lwip / Mbed TLS trees (Spikes A3, B, B2)"
for d in unikraft lib-lwip; do
  printf '%-16s HEAD=%s dirty=%s\n' "$d" \
    "$(git -C "$W/$d" rev-parse HEAD 2>&1)" \
    "$(git -C "$W/$d" status --porcelain 2>/dev/null | wc -l)"
done
echo "--- unikraft worktrees"
git -C "$W/unikraft" worktree list 2>&1
echo "--- lib-lwip worktrees"
git -C "$W/lib-lwip" worktree list 2>&1
echo "--- Spike B worktree unikraft-igkmd (must be 0001+0002 only)"
git -C "$W/unikraft-igkmd" diff --stat 2>&1
echo "--- Spike B2 worktrees unikraft-igkme (0001+0002) and lib-lwip-igkme (0003)"
git -C "$W/unikraft-igkme" diff --stat 2>&1
git -C "$W/lib-lwip-igkme" diff --stat 2>&1
ls -d "$W"/* 2>&1
sha256sum "$W/mbedtls/mbedtls-3.6.7.tar.bz2" 2>&1
ls "$W/tools/prefix/bin" 2>&1
echo "--- app-elfloader / lib-libelf clones (created by increment-f build-guest.sh if absent)"
for d in app-elfloader lib-libelf; do
  if [[ -d "$W/$d/.git" ]]; then
    printf '%-14s HEAD=%s dirty=%s\n' "$d" "$(git -C "$W/$d" rev-parse HEAD)" "$(git -C "$W/$d" status --porcelain | wc -l)"
  else
    printf '%-14s absent\n' "$d"
  fi
done
du -sh "$SCRATCH" "$W" 2>/dev/null
df -h "$HOME" | tail -n 1

section "host C toolchain + glibc for the UNMODIFIED Linux test binary"
gcc --version 2>&1 | head -1
ld --version 2>&1 | head -1
ldd --version 2>&1 | head -1
getconf GNU_LIBC_VERSION 2>&1
dpkg-query -W -f='${Package} ${Version}\n' gcc libc6 libc6-dev binutils 2>&1
for f in rcrt1.o crt1.o libc.a; do
  printf '%-8s %s\n' "$f" "$(gcc -print-file-name=$f)"
done
[[ -f "$(gcc -print-file-name=rcrt1.o)" ]] || { echo "NOTE: no rcrt1.o -> gcc -static-pie unavailable"; }
file "$(gcc -print-file-name=libc.a)" 2>&1

section "fetch tools / optional stretch toolchains (presence only)"
for t in git wget curl tar xz bzip2 file readelf objdump strings go busybox python3 cpio; do
  printf '%-8s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
[[ -x /usr/bin/busybox ]] && file /usr/bin/busybox
command -v go >/dev/null && go version

section "tap name / subnet / port availability (increment-f: 192.168.204.0/24, taps igkf<ddHHMMSS>)"
echo "igkf* links: [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igk' | tr '\n' ' ')]"
echo "192.168.204.x on host: $(ip -4 -br addr | grep -c '192\.168\.204\.')"
ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|645[01]|7100)\b' || echo "no listener on 5001/6443-6451/7100"
echo "firecracker processes: [$(pgrep -a firecracker | tr '\n' ';')]"

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
