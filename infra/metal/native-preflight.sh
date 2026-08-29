#!/usr/bin/env bash
# Fail-closed native x86_64/KVM qualification used by every metal Run.
set -euo pipefail

fatal() { echo "FATAL: native metal preflight: $*" >&2; exit 1; }

CPUINFO="${OVERDRIVE_PREFLIGHT_CPUINFO:-/proc/cpuinfo}"
HYPERVISOR_TYPE="${OVERDRIVE_PREFLIGHT_HYPERVISOR_TYPE:-/sys/hypervisor/type}"
KVM_DEVICE="${OVERDRIVE_PREFLIGHT_KVM_DEVICE:-/dev/kvm}"
CGROUP_CONTROLLERS="${OVERDRIVE_PREFLIGHT_CGROUP_CONTROLLERS:-/sys/fs/cgroup/cgroup.controllers}"
DETECT_VIRT="${OVERDRIVE_PREFLIGHT_DETECT_VIRT:-systemd-detect-virt}"
CLOUD_HYPERVISOR="${OVERDRIVE_PREFLIGHT_CLOUD_HYPERVISOR:-cloud-hypervisor}"
OWNER_PATH="${OVERDRIVE_METAL_OWNER_PATH:-/run/lock/overdrive-metal-shared.owner}"

[ "${OVERDRIVE_PREFLIGHT_ARCH:-$(uname -m)}" = "x86_64" ] \
  || fatal "architecture must be literal x86_64"
command -v "${DETECT_VIRT}" >/dev/null 2>&1 || fatal "systemd-detect-virt is required"
set +e
VIRT="$(${DETECT_VIRT} 2>/dev/null)"
VIRT_STATUS=$?
set -e
[ "${VIRT_STATUS}" -eq 1 ] && [ "${VIRT}" = "none" ] \
  || fatal "host reports virtualization (${VIRT:-unknown}, status=${VIRT_STATUS})"
! grep -qw hypervisor "${CPUINFO}" || fatal "CPU hypervisor flag is present"
[ ! -s "${HYPERVISOR_TYPE}" ] || fatal "/sys/hypervisor/type reports a hypervisor"
grep -qE '^flags.*\b(vmx|svm)\b' "${CPUINFO}" || fatal "CPU exposes neither vmx nor svm"
if [ "${OVERDRIVE_PREFLIGHT_KVM_CHARACTER:-}" != "yes" ]; then
  [ -c "${KVM_DEVICE}" ] || fatal "/dev/kvm is not a character device"
fi

if [ -n "${OVERDRIVE_PREFLIGHT_KVM_PROBE:-}" ]; then
  "${OVERDRIVE_PREFLIGHT_KVM_PROBE}" "${KVM_DEVICE}" || fatal "KVM API/open/create-VM probe failed"
else
  python3 - "${KVM_DEVICE}" <<'KVM_PY'
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

[ -f "${CGROUP_CONTROLLERS}" ] || fatal "cgroup v2 controllers are unavailable"
command -v "${CLOUD_HYPERVISOR}" >/dev/null 2>&1 || fatal "cloud-hypervisor is unavailable"
[ -z "${OVERDRIVE_METAL_KERNEL:-}" ] || [ -f "${OVERDRIVE_METAL_KERNEL}" ] \
  || fatal "selected guest kernel does not exist"
[ -z "${OVERDRIVE_METAL_ROOTFS:-}" ] || [ -f "${OVERDRIVE_METAL_ROOTFS}" ] \
  || fatal "selected guest rootfs does not exist"

grep -qx "token=${OVERDRIVE_EXPECTED_TOKEN}" "${OWNER_PATH}" \
  || fatal "the active lease owner token changed"
MARKER="$(cat "${OVERDRIVE_REMOTE_DIR}/.overdrive-metal-source" 2>/dev/null || true)"
EXPECTED="commit=${OVERDRIVE_EXPECTED_COMMIT}
workspace=${OVERDRIVE_EXPECTED_WORKSPACE}
source_digest=${OVERDRIVE_EXPECTED_SOURCE}"
[ "${MARKER}" = "${EXPECTED}" ] || fatal "runtime source marker is stale or mismatched"
