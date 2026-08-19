#!/usr/bin/env bash
# Root-mode provisioning for the BARE-METAL box.
#
# NOTE ON SCOPE: this file is written to be reusable by the Lima dev VM, but
# Lima does NOT currently invoke it — infra/lima/overdrive-dev.yaml still
# carries its own inline provision blocks. So this is not yet a shared SSOT,
# and the two WILL drift until one calls the other. Known divergences today:
# Lima additionally installs kernel-matched linux-tools + a bpftool symlink,
# a /dev/kvm 0666 udev rule (deliberately NOT replicated here — see
# infra/metal/provision.sh), an ip_forward sysctl, wasmtime, pwru and kraft.
#
# Idempotent: safe to re-run against a live host.
#
# Usage:  sudo infra/provision/common-system.sh
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck disable=SC1091
set -a; . "${HERE}/versions.env"; set +a

log() { printf '\n=== [common-system] %s\n' "$*"; }

[ "$(id -u)" -eq 0 ] || { echo "must run as root" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Packages. Same base list as infra/lima/overdrive-dev.yaml, plus xfsprogs,
# rsync and smartmontools which the metal box needs and Lima does not.
#
# NOTE: no `qemu-kvm` — it is a defunct transitional package with no install
# candidate on Ubuntu 26.04, and apt is transactional, so naming it aborts the
# WHOLE install. KVM comes from the kernel; qemu-system-* provides the tooling.
#
# qemu-system-* is not optional on this box: the spike's cross-VMM population
# diff (CH vs QEMU on an identical image) is what distinguishes an environment
# fault from a Cloud Hypervisor fault. Keep it installed.
# ---------------------------------------------------------------------------
log "apt packages"
export DEBIAN_FRONTEND=noninteractive
apt-get -o DPkg::Lock::Timeout=600 update
apt-get -o DPkg::Lock::Timeout=600 install -y --no-install-recommends \
    build-essential pkg-config ca-certificates curl git jq \
    unzip xz-utils cpio bzip2 zstd file \
    clang lld mold llvm libclang-dev \
    libelf-dev libbpf-dev linux-libc-dev \
    linux-tools-common linux-tools-generic \
    zlib1g-dev libssl-dev libzstd-dev libudev-dev \
    qemu-system-x86 qemu-system-arm qemu-utils \
    virtiofsd \
    xdp-tools bpfcc-tools \
    tcpdump iproute2 bridge-utils \
    python3 python3-venv python3-pip pipx \
    e2fsprogs xfsprogs rsync smartmontools

# ---------------------------------------------------------------------------
# Cloud Hypervisor — single static binary from the upstream release.
# ---------------------------------------------------------------------------
log "cloud-hypervisor ${CLOUD_HYPERVISOR_VERSION}"
ARCH="$(uname -m)"
case "${ARCH}" in
  x86_64)  CH_ASSET="cloud-hypervisor-static" ;;
  aarch64) CH_ASSET="cloud-hypervisor-static-aarch64" ;;
  *) echo "unsupported arch: ${ARCH}" >&2; exit 1 ;;
esac

if ! command -v cloud-hypervisor >/dev/null 2>&1 \
   || ! cloud-hypervisor --version 2>&1 | grep -qF "${CLOUD_HYPERVISOR_VERSION#v}"; then
  curl -fsSL -o /usr/local/bin/cloud-hypervisor \
    "https://github.com/cloud-hypervisor/cloud-hypervisor/releases/download/${CLOUD_HYPERVISOR_VERSION}/${CH_ASSET}"
  chmod +x /usr/local/bin/cloud-hypervisor
fi
cloud-hypervisor --version

# Fail loudly if the build lacks Landlock. P5's confinement work depends on it,
# and discovering its absence at probe time reads as an unrelated failure.
if ! cloud-hypervisor --help 2>&1 | grep -q -- '--landlock'; then
  echo "FATAL: this cloud-hypervisor build has no --landlock; P5 cannot run" >&2
  exit 1
fi

log "virtiofsd"
[ -x "${VIRTIOFSD_BIN}" ] || { echo "FATAL: ${VIRTIOFSD_BIN} missing" >&2; exit 1; }
"${VIRTIOFSD_BIN}" --version

log "done"
