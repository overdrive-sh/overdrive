#!/usr/bin/env bash
# Fail-closed native x86_64/KVM qualification used by every metal Run.
set -euo pipefail

fatal() {
  local probe="$1"
  printf 'OVERDRIVE_METAL_RUNNER_ERROR error=preflight_failed probe=%s\n' "$probe" >&2
  exit 78
}

CPUINFO="${OVERDRIVE_PREFLIGHT_CPUINFO:-/proc/cpuinfo}"
HYPERVISOR_TYPE="${OVERDRIVE_PREFLIGHT_HYPERVISOR_TYPE:-/sys/hypervisor/type}"
KVM_DEVICE="${OVERDRIVE_PREFLIGHT_KVM_DEVICE:-/dev/kvm}"
CGROUP_CONTROLLERS="${OVERDRIVE_PREFLIGHT_CGROUP_CONTROLLERS:-/sys/fs/cgroup/cgroup.controllers}"
DETECT_VIRT="${OVERDRIVE_PREFLIGHT_DETECT_VIRT:-systemd-detect-virt}"
CLOUD_HYPERVISOR="${OVERDRIVE_PREFLIGHT_CLOUD_HYPERVISOR:-cloud-hypervisor}"
OWNER_PATH="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"

[ "${OVERDRIVE_PREFLIGHT_ARCH:-$(uname -m)}" = "x86_64" ] || fatal arch
command -v "${DETECT_VIRT}" >/dev/null 2>&1 || fatal virtualization
set +e
VIRT="$(${DETECT_VIRT} 2>/dev/null)"
VIRT_STATUS=$?
set -e
[ "${VIRT_STATUS}" -eq 1 ] && [ "${VIRT}" = "none" ] \
  || fatal virtualization
! grep -qw hypervisor "${CPUINFO}" || fatal cpu
[ ! -s "${HYPERVISOR_TYPE}" ] || fatal virtualization
grep -qE '^flags.*\b(vmx|svm)\b' "${CPUINFO}" || fatal cpu
if [ "${OVERDRIVE_PREFLIGHT_KVM_CHARACTER:-}" != "yes" ]; then
  [ -c "${KVM_DEVICE}" ] || fatal kvm_device
fi

if [ -n "${OVERDRIVE_PREFLIGHT_KVM_PROBE:-}" ]; then
  "${OVERDRIVE_PREFLIGHT_KVM_PROBE}" "${KVM_DEVICE}" || fatal kvm_api
else
  python3 - "${KVM_DEVICE}" <<'KVM_PY' || fatal kvm_api
import fcntl
import os
import sys

fd = os.open(sys.argv[1], os.O_RDWR | os.O_CLOEXEC)
try:
    version = fcntl.ioctl(fd, 0xAE00, 0)
    if version != 12:
        raise SystemExit(f"KVM_GET_API_VERSION returned {version}, expected 12")
    vm_fd = fcntl.ioctl(fd, 0xAE01, 0)
    os.close(vm_fd)
finally:
    os.close(fd)
KVM_PY
fi

[ -f "${CGROUP_CONTROLLERS}" ] || fatal cgroup_v2
command -v "${CLOUD_HYPERVISOR}" >/dev/null 2>&1 || fatal vmm
[ -n "${OVERDRIVE_METAL_KERNEL:-}" ] || fatal guest_kernel
[ -f "${OVERDRIVE_METAL_KERNEL}" ] && [ -r "${OVERDRIVE_METAL_KERNEL}" ] \
  || fatal guest_kernel
[ -n "${OVERDRIVE_METAL_ROOTFS:-}" ] || fatal guest_rootfs
[ -f "${OVERDRIVE_METAL_ROOTFS}" ] && [ -r "${OVERDRIVE_METAL_ROOTFS}" ] \
  || fatal guest_rootfs

grep -qx "token=${OVERDRIVE_EXPECTED_TOKEN}" "${OWNER_PATH}" \
  || fatal owner
MARKER="$(cat "${OVERDRIVE_REMOTE_DIR}/.overdrive-metal-source" 2>/dev/null || true)"
EXPECTED="commit=${OVERDRIVE_EXPECTED_COMMIT}
workspace=${OVERDRIVE_EXPECTED_WORKSPACE}
source_digest=${OVERDRIVE_EXPECTED_SOURCE}"
[ "${MARKER}" = "${EXPECTED}" ] || fatal source
