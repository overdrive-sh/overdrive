#!/usr/bin/env bash
# Read-only host inventory for Spike D (GH #303, increment-i). Creates nothing,
# installs nothing, touches no kernel. Substrate probes (testing.md, fail
# closed) + the two feasibility gates for an OOT kernel module in a stock Linux
# guest on Firecracker:
#   Gate A: a Firecracker-bootable x86_64 vmlinux ELF stock kernel with
#           CONFIG_TLS + CONFIG_VSOCKETS, obtainable without building a kernel.
#   Gate B: matching kernel build headers / Module.symvers to build an OOT .ko
#           whose vermagic matches that guest kernel.
set -uo pipefail

section() { printf '\n=== %s ===\n' "$*"; }
fail=0
SCRATCH="$HOME/igkm-spike-a"

section "substrate probes (testing.md, fail closed)"
printf 'uname -a: %s\n' "$(uname -a)"
printf 'uname -m: %s\n' "$(uname -m)"
[[ "$(uname -m)" == x86_64 ]] || { echo "FAIL: not x86_64"; fail=1; }
printf 'uname -r: %s\n' "$(uname -r)"
virt="$(systemd-detect-virt 2>&1)"; vstat=$?
printf 'systemd-detect-virt: %s (exit %s)\n' "$virt" "$vstat"
[[ "$virt" == none ]] || { echo "FAIL: virtualized host (no-nesting trust boundary)"; fail=1; }
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
echo "nproc=$(nproc)"
df -h "$HOME" | tail -n 1

section "Firecracker v1.17.0 (Spike A2 binary, reuse)"
FC="$SCRATCH/firecracker/v1.17.0/release-v1.17.0-x86_64/firecracker-v1.17.0-x86_64"
if [[ -x "$FC" ]]; then
  sha256sum "$FC"
  "$FC" --version 2>&1 | head -1
else
  echo "NOTE: Firecracker binary absent at $FC"
fi

section "HOST running kernel: kTLS + vsock config + module availability"
# Read-only. The HOST kernel is a reference for what a stock Ubuntu kernel
# carries; it is NEVER the guest kernel unless Firecracker can boot it.
KREL="$(uname -r)"
for cfg in /boot/config-"$KREL" /proc/config.gz; do
  if [[ -r "$cfg" ]]; then
    echo "--- $cfg ---"
    { [[ "$cfg" == *.gz ]] && zcat "$cfg" || cat "$cfg"; } \
      | grep -E '^CONFIG_(TLS|TLS_DEVICE|VSOCKETS|VSOCKETS_LOADABLE|VIRTIO_VSOCKETS|VHOST_VSOCK|VIRTIO_MMIO|VIRTIO_BLK|VIRTIO_NET)\b' \
      | sort
    break
  fi
done
echo "--- loadable tls / vsock modules present on host (NOT loaded by us) ---"
find /lib/modules/"$KREL"/kernel/net/tls /lib/modules/"$KREL"/kernel/net/vmw_vsock -name '*.ko*' 2>/dev/null
echo "modinfo tls vermagic: $(modinfo -F vermagic tls 2>/dev/null || echo 'n/a')"
echo "modinfo vsock vermagic: $(modinfo -F vermagic vsock 2>/dev/null || echo 'n/a')"

section "GATE B: kernel build headers for OOT module (host kernel)"
BUILD="/lib/modules/$KREL/build"
echo "build symlink: $(ls -ld "$BUILD" 2>&1)"
if [[ -e "$BUILD/Makefile" ]]; then
  echo "has Makefile: yes"
  echo "Module.symvers: $(ls -l "$BUILD/Module.symvers" 2>&1 | head -1)"
  echo ".config: $(ls -l "$BUILD/.config" 2>&1 | head -1)"
  echo "include/generated/utsrelease.h: $(cat "$BUILD/include/generated/utsrelease.h" 2>/dev/null | tr -d '\n')"
else
  echo "has Makefile: NO (no headers for host kernel) -> OOT build against host kernel needs a download"
fi
ls -d /usr/src/linux-headers-* 2>/dev/null || echo "no /usr/src/linux-headers-*"

section "GATE A: any Firecracker-bootable Linux vmlinux ELF already in scratch?"
# Firecracker x86_64 requires an uncompressed ELF vmlinux, not a distro bzImage.
found_vmlinux=0
while IFS= read -r f; do
  [[ -z "$f" ]] && continue
  found_vmlinux=1
  printf '%s  %s\n' "$(file -b "$f" | cut -c1-60)" "$f"
done < <(find "$SCRATCH" "$HOME/overdrive/spike-scratch/in-guest-kernel-mtls" -maxdepth 6 \
          \( -name 'vmlinux*' -o -name 'bzImage*' -o -name 'vmlinuz*' -o -name '*.bin' \) \
          -type f 2>/dev/null | head -40)
[[ "$found_vmlinux" -eq 0 ]] && echo "none found in scratch/overdrive trees"

section "GATE A: outbound network to fetch a prebuilt stock kernel (presence only)"
for host in s3.amazonaws.com mirrors.ubuntu.com cdn.kernel.org github.com; do
  if timeout 6 bash -c "echo >/dev/tcp/$host/443" 2>/dev/null; then
    echo "reachable: $host:443"
  else
    echo "UNREACHABLE: $host:443"
  fi
done

section "host C / OOT-module build toolchain (presence only, NO installs)"
for t in gcc make ld flex bison bc cpio gzip xz zstd busybox qemu-img mkfs.ext4 \
         depmod file readelf objdump strings curl wget tar python3 rustc cargo; do
  printf '%-10s %s\n' "$t" "$(command -v "$t" 2>/dev/null || echo MISSING)"
done
gcc --version 2>&1 | head -1
echo "kernel-build prerequisite packages (query only):"
dpkg-query -W -f='${Package} ${Version}\n' build-essential libelf-dev linux-headers-"$KREL" 2>&1 | sed 's/^/  /'

section "reusable Spike C scratch (igkmh out/certs, FC, staged kernel, CARGO_HOME) + any leftover from an interrupted Spike D attempt"
echo "spike-private CARGO_HOME: $(ls -d "$SCRATCH"/cargo-home 2>/dev/null || echo 'absent')"
echo "Spike C out dir (igkmh): $(ls -d "$SCRATCH"/igkmh/out 2>/dev/null || echo 'absent')"
echo "Spike C staged kernel: $(ls -l "$SCRATCH"/igkmh/kernel/vmlinuz-host 2>/dev/null || echo 'absent')"
echo "Spike D out dir (igkmd, this increment -- expected ABSENT until build-all): $(ls -d "$SCRATCH"/igkmd/out 2>/dev/null || echo 'absent (good; clean start)')"
echo "Spike D host target (igkmd-host-target -- expected ABSENT): $(ls -d "$SCRATCH"/igkmd-host-target 2>/dev/null || echo 'absent (good; clean start)')"

section "port / subnet / tap availability (increment-i plans 192.168.204.0/24 backends :6443/:6444, VIPs 10.80.0.x, taps igkd<...>)"
echo "igk* links (igkd/igkh -- must be none): [$(ip -o link show 2>/dev/null | sed -E 's/^[0-9]+: ([^:@]+).*/\1/' | grep -E '^igk[dh]' | tr '\n' ' ')]"
echo "192.168.204.x on host: $(ip -4 -br addr | grep -c '192\.168\.204\.')"
echo "10.80.0.x on host (VIP subnet -- must be 0): $(ip -4 -br addr | grep -c '10\.80\.0\.')"
ss -Htlnp 2>/dev/null | grep -E ':(5001|644[3-9]|645[012]|7100)\b' || echo "no listener on 5001/6443-6452/7100"
echo "firecracker procs: [$(pgrep -a firecracker | tr '\n' ';')]"

section "GATE E: kernel header signatures the Spike E accept()/non-blocking hooks depend on"
# Read-only grep of the stock kernel's own headers so the module's accept hook
# prototype, the proto_accept_arg layout, the inet_sock address fields, and the
# proto_ops.poll signature are written against GROUND TRUTH, not guessed
# (.claude/rules/debugging.md §9). Nothing is built or installed here.
KREL="$(uname -r)"
KH="/lib/modules/$KREL/build"
echo "kernel build tree: $KH"
echo "--- struct proto .accept member (include/net/sock.h) ---"
grep -n -A1 'struct sock \*\s*(\*accept)' "$KH/include/net/sock.h" 2>/dev/null || echo "  (accept member not matched; dumping 'accept' lines)"
grep -n 'accept' "$KH/include/net/sock.h" 2>/dev/null | head -8
echo "--- struct proto_accept_arg (search headers) ---"
grep -rn 'struct proto_accept_arg' "$KH/include/" 2>/dev/null | head
grep -rn -A8 'struct proto_accept_arg {' "$KH/include/" 2>/dev/null | head -20
echo "--- inet_csk_accept prototype (include/net/inet_connection_sock.h) ---"
grep -n -A2 'inet_csk_accept' "$KH/include/net/inet_connection_sock.h" 2>/dev/null | head
echo "--- struct proto_ops .poll member (include/linux/net.h) ---"
grep -n -A1 '(\*poll)' "$KH/include/linux/net.h" 2>/dev/null | head
echo "--- inet_sk address fields (include/net/inet_sock.h): inet_saddr/inet_daddr/inet_dport/inet_num/inet_sport ---"
grep -n -E 'inet_(saddr|daddr|dport|num|sport|rcv_saddr)\b' "$KH/include/net/inet_sock.h" 2>/dev/null | head -20
echo "--- struct sock callback fields we wake (sk_state_change/sk_write_space/sk_data_ready/sk_error_report/sk_err) ---"
grep -n -E 'sk_(state_change|write_space|data_ready|error_report|err)\b' "$KH/include/net/sock.h" 2>/dev/null | head -20
echo "--- kthread + waitqueue + EPOLL masks present? ---"
echo "kthread.h: $(ls "$KH/include/linux/kthread.h" 2>/dev/null || echo MISSING)"
echo "wait.h:    $(ls "$KH/include/linux/wait.h" 2>/dev/null || echo MISSING)"
echo "poll.h:    $(ls "$KH/include/linux/poll.h" 2>/dev/null || echo MISSING)"
grep -rn -E '\bEPOLL(IN|OUT|ERR|HUP|RDNORM|WRNORM)\b' "$KH/include/uapi/linux/eventpoll.h" 2>/dev/null | head
echo "--- tcp_poll + inet_stream_ops + inet_csk_accept + kallsyms symbols (kallsyms presence) ---"
for sym in tcp_poll inet_stream_ops inet_csk_accept tcp_prot tcp_set_ulp kallsyms_lookup_name; do
  printf '  %-22s %s\n' "$sym" "$(grep -m1 -E " $sym\$" /proc/kallsyms 2>/dev/null | awk '{print $2}' || echo '?')"
done
echo "--- does the stock kernel export a usable sk_prot->accept on tcp_prot? (modinfo not needed; symbol presence above) ---"

section "verdict"
if [[ "$fail" -eq 0 ]]; then echo "SUBSTRATE OK"; else echo "SUBSTRATE FAILED"; exit 1; fi
