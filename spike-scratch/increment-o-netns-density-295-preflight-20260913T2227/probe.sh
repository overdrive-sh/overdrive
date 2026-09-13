#!/usr/bin/env bash
# Throwaway Phase-1 substrate probe for GH #295. No production changes.
# The mechanism cannot be interpreted until the required substrate exists.
# Run from the repository root via cargo xtask lima run -- bash <this-file>.
set -uo pipefail

spike_here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
spike_evidence=$(mktemp -d "$spike_here/evidence-XXXXXXXX") || exit 1
exec > >(tee -a "$spike_evidence/preflight.log") 2>&1
spike_start=$(python3 -c 'import time; print(time.perf_counter())')

record() {
  printf '\nCOMMAND'
  printf ' %q' "$@"
  printf '\nSTART %s\n' "$(date -u +%FT%TZ)"
  "$@"
  spike_rc=$?
  printf 'COMPLETE %s EXIT_STATUS=%s\n' "$(date -u +%FT%TZ)" "$spike_rc"
}

printf 'GH295_PHASE1_SUBSTRATE_PROBE\nEVIDENCE=%s\n' "$spike_evidence"
record uname -r
record uname -m
record systemd-detect-virt
record id
record cloud-hypervisor --version
record ls -l /dev/kvm
record ls -l /boot
record ls -l /lib/modules
record ip -br link
record ip netns list
record nft --version

spike_kernel=$(uname -r)
spike_arch=$(uname -m)
spike_virtualization=$(systemd-detect-virt)
case "$spike_kernel" in
  6.18|6.18.*|6.18-*) printf 'PINNED_KERNEL_MATCH=yes\n' ;;
  *) printf 'PINNED_KERNEL_MATCH=no REQUIRED=6.18_LTS ACTUAL=%s\n' "$spike_kernel" ;;
esac
printf 'NATIVE_GUEST_EVIDENCE_REQUIREMENT=x86_64_nonvirtualized\n'
printf 'ACTUAL_ARCH=%s ACTUAL_VIRTUALIZATION=%s\n' "$spike_arch" "$spike_virtualization"
printf 'MECHANISM_EXECUTED=no\nMICROVM_BOOTED=no\nWIRE_CAPTURE=none\n'
printf 'STOP_REASON=required_pinned_kernel_and_allowed_real_guest_substrate_unavailable_in_this_Lima_instance\n'
python3 -c 'import sys,time; print(f"PREFLIGHT_ELAPSED_SECONDS={time.perf_counter()-float(sys.argv[1]):.6f}")' "$spike_start"
printf 'PROBE_OUTCOME=BLOCKED_BEFORE_MECHANISM\n'
exit 2
