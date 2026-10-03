#!/usr/bin/env bash
# Read-only host inventory for Spike A3 (Unikraft on Firecracker): substrate
# probes (testing.md, fail closed), the Firecracker binaries fetched by Spike A2,
# and the user-space build tooling a Unikraft make/Kconfig build needs. Creates
# nothing.
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

section "Firecracker binaries from Spike A2 (must exist, unchanged)"
for v in v1.17.0 v1.11.0; do
  bin="$SCRATCH/firecracker/$v/release-$v-x86_64/firecracker-$v-x86_64"
  sha256sum "$bin" 2>&1
  "$bin" --version 2>&1 | head -1
done

section "identity / scratch"
id
echo "HOME=$HOME"
df -h "$HOME" | tail -1
ls -la "$SCRATCH" 2>&1 | head -20
ls -la "$SCRATCH/unikraft" 2>&1 | head

section "build tooling (Unikraft make/Kconfig build)"
for t in gcc cc make ld as ar objcopy strip nm readelf git curl wget python3 \
         flex bison m4 unzip patch gawk awk sed tar gzip xz bc cpio kraft nasm; do
  printf '%-10s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
gcc --version 2>/dev/null | head -1
make --version 2>/dev/null | head -1
ld --version 2>/dev/null | head -1
python3 --version
wget --version 2>/dev/null | head -1
printf 'nasm (Spike A user-space build): %s\n' "$("$SCRATCH/bin/nasm" -v 2>&1 || echo MISSING)"
printf 'libncurses headers: %s\n' "$(ls /usr/include/ncurses.h 2>/dev/null || echo MISSING)"

section "network reachability (HEAD/metadata only)"
for u in https://github.com https://ftp.gnu.org/gnu/ https://codeload.github.com; do
  printf '%s -> %s\n' "$u" "$(curl -sS -o /dev/null -m 15 -w '%{http_code}' "$u" 2>&1)"
done

section "tap name / subnet availability"
echo "igkmc0 present: $(ip link show igkmc0 >/dev/null 2>&1 && echo yes || echo no)"
echo "192.168.203.x on host: $(ip -4 -br addr | grep -c '192\.168\.203\.')"
ip -4 -br addr

section "staging files for preflight"
ls -l /srv/vm/overdrive-testing/kernel /srv/vm/overdrive-testing/rootfs.ext4 2>&1

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
