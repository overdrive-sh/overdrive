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
set -a
# shellcheck disable=SC1091
. "${HERE}/versions.env"
set +a

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
    "pahole=${PAHOLE_PACKAGE_VERSION}" \
    linux-tools-common linux-tools-generic \
    zlib1g-dev libssl-dev libzstd-dev libudev-dev \
    qemu-system-x86 qemu-system-arm qemu-utils \
    virtiofsd \
    xdp-tools bpfcc-tools \
    tcpdump iproute2 bridge-utils \
    python3 python3-venv python3-pip pipx \
    e2fsprogs xfsprogs rsync smartmontools

# Ubuntu's sparse 0.6.4 package cannot parse the __typeof_unqual__ probe
# required by the pinned Linux 7.0 checker. Install the validated upstream
# revision in /usr/local/bin, ahead of the distro copy.
log "sparse ${SPARSE_COMMIT}"
SPARSE_SOURCE="$(mktemp -d)"
trap 'rm -rf "${SPARSE_SOURCE}"' EXIT
git -C "${SPARSE_SOURCE}" init
git -C "${SPARSE_SOURCE}" remote add origin \
  https://git.kernel.org/pub/scm/devel/sparse/sparse.git
git -C "${SPARSE_SOURCE}" fetch --depth=1 origin "${SPARSE_COMMIT}"
git -C "${SPARSE_SOURCE}" checkout --detach FETCH_HEAD
test "$(git -C "${SPARSE_SOURCE}" rev-parse HEAD)" = "${SPARSE_COMMIT}"
make -C "${SPARSE_SOURCE}" LLVM_CONFIG=/bin/false sparse
install -m 0755 "${SPARSE_SOURCE}/sparse" /usr/local/bin/sparse
sparse --version
rm -rf "${SPARSE_SOURCE}"
trap - EXIT

# Cloud Hypervisor is intentionally not installed in root mode. The pinned
# fork is built from source and checksum-gated by common-user.sh after the
# user's Rust toolchain and native musl compiler are available.
log "cloud-hypervisor installation deferred to common-user.sh"

log "virtiofsd"
[ -x "${VIRTIOFSD_BIN}" ] || { echo "FATAL: ${VIRTIOFSD_BIN} missing" >&2; exit 1; }
"${VIRTIOFSD_BIN}" --version

log "done"
