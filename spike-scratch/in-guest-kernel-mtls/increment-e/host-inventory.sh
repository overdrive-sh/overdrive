#!/usr/bin/env bash
# Read-only host inventory for Spike B2 (GH #303, increment-e): substrate
# probes (testing.md, fail closed), the Firecracker binary, the Unikraft /
# lib-lwip / Mbed TLS trees Spikes A3 and B left in ~/igkm-spike-a, the
# spike-private CARGO_HOME, and the capture tooling. Creates nothing.
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

section "Unikraft / lib-lwip / Mbed TLS trees (Spikes A3 and B)"
for d in unikraft lib-lwip; do
  printf '%-16s HEAD=%s dirty=%s\n' "$d" \
    "$(git -C "$W/$d" rev-parse HEAD 2>&1)" \
    "$(git -C "$W/$d" status --porcelain 2>/dev/null | wc -l)"
done
echo "--- unikraft worktrees"
git -C "$W/unikraft" worktree list 2>&1
echo "--- Spike B worktree unikraft-igkmd: diff --stat vs its HEAD (must be patches 0001+0002 only)"
git -C "$W/unikraft-igkmd" rev-parse HEAD 2>&1
git -C "$W/unikraft-igkmd" diff --stat 2>&1
echo "--- lib-lwip worktrees"
git -C "$W/lib-lwip" worktree list 2>&1
ls -d "$W"/* 2>&1
ls -l "$W/mbedtls" 2>&1
sha256sum "$W/mbedtls/mbedtls-3.6.7.tar.bz2" 2>&1
ls "$W/tools/prefix/bin" 2>&1
du -sh "$SCRATCH" "$W" 2>/dev/null

section "Rust toolchain + spike-private CARGO_HOME"
"$HOME/.cargo/bin/rustc" --version 2>&1
"$HOME/.cargo/bin/cargo" --version 2>&1
du -sh "$SCRATCH/cargo-home" 2>/dev/null
ls -d "$SCRATCH"/cargo-home/registry/src/*/ring-0.17.14 "$SCRATCH"/cargo-home/registry/src/*/rustls-0.23.45 2>&1

section "capture and diagnostic tooling"
for t in tcpdump ss ip python3 openssl gcc make curl bzip2 tar; do
  printf '%-8s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
tcpdump --version 2>&1 | head -2
python3 --version

section "tap name / subnet / port availability (increment-e uses igkme0, 192.168.204.0/24)"
echo "igkme0 present: $(ip link show igkme0 >/dev/null 2>&1 && echo yes || echo no)"
echo "192.168.204.x on host: $(ip -4 -br addr | grep -c '192\.168\.204\.')"
ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|6450)\b' || echo "no listener on 5001/6443-6450"
echo "firecracker processes: [$(pgrep -a firecracker | tr '\n' ';')]"

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
