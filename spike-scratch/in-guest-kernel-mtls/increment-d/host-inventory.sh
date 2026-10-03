#!/usr/bin/env bash
# Read-only host inventory for Spike B (in-guest mTLS hook in Unikraft on
# Firecracker): substrate probes (testing.md, fail closed), the Firecracker
# binary and Unikraft tree Spike A3 left, the Rust toolchain for the host
# stubs, and the capture tooling. Creates nothing.
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

section "Firecracker v1.17.0 from Spike A2 (must exist, unchanged)"
bin="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
sha256sum "$bin" 2>&1
"$bin" --version 2>&1 | head -1

section "Spike A3 Unikraft tree"
for d in unikraft lib-lwip; do
  printf '%-10s HEAD=%s tree=%s dirty=%s\n' "$d" \
    "$(git -C "$SCRATCH/unikraft/$d" rev-parse HEAD 2>&1)" \
    "$(git -C "$SCRATCH/unikraft/$d" rev-parse 'HEAD^{tree}' 2>&1)" \
    "$(git -C "$SCRATCH/unikraft/$d" status --porcelain 2>/dev/null | wc -l)"
done
git -C "$SCRATCH/unikraft/unikraft" worktree list 2>&1
ls "$SCRATCH/unikraft/tools/prefix/bin" 2>&1
du -sh "$SCRATCH" "$SCRATCH/unikraft" 2>/dev/null

section "Rust toolchain for the host stubs (login user)"
echo "PATH=$PATH"
for t in cargo rustc rustup; do
  printf '%-7s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo 'not on PATH')"
done
ls -l "$HOME/.cargo/bin" 2>&1 | head -20
"$HOME/.cargo/bin/rustc" --version 2>&1
"$HOME/.cargo/bin/cargo" --version 2>&1
"$HOME/.cargo/bin/rustup" show active-toolchain 2>&1
du -sh "$HOME/.cargo/registry" 2>/dev/null

section "capture and diagnostic tooling"
for t in tcpdump ss ip python3 openssl gcc make curl bzip2 tar; do
  printf '%-8s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
tcpdump --version 2>&1 | head -2
python3 --version

section "network reachability (HEAD/metadata only)"
for u in https://index.crates.io/config.json https://static.crates.io https://github.com; do
  printf '%s -> %s\n' "$u" "$(curl -sS -o /dev/null -m 15 -w '%{http_code}' "$u" 2>&1)"
done

section "tap name / subnet / port availability"
echo "igkmd0 present: $(ip link show igkmd0 >/dev/null 2>&1 && echo yes || echo no)"
echo "192.168.203.x on host: $(ip -4 -br addr | grep -c '192\.168\.203\.')"
ss -Htlnp 2>/dev/null | grep -E ':(5001|6443|6444|6445|6446)\b' || echo "no listener on 5001/6443-6446"

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
