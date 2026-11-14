#!/usr/bin/env bash
# Print the pinned Cloud Hypervisor fork identity as `key=value` lines for
# $GITHUB_OUTPUT. The SSOT is the `param:` block of infra/lima/overdrive-dev.yaml;
# nothing here restates a value. x86_64 only (the only CI runner arch).
set -euo pipefail
yaml="${1:-infra/lima/overdrive-dev.yaml}"
pin() {
  local v
  v="$(sed -n "s/^  $1: *\"\\(.*\\)\"[[:space:]]*\$/\\1/p" "${yaml}")"
  [ -n "${v}" ] || { echo "FATAL: ${1} not found in ${yaml}" >&2; exit 1; }
  printf '%s' "${v}"
}
rev="$(pin CLOUD_HYPERVISOR_FORK_REV)"
toolchain="$(pin CLOUD_HYPERVISOR_BUILD_TOOLCHAIN)"
sha="$(pin CLOUD_HYPERVISOR_SHA256_X86_64)"
repo="$(pin CLOUD_HYPERVISOR_FORK_REPO)"
echo "repo=${repo}"
echo "rev=${rev}"
echo "toolchain=${toolchain}"
echo "sha256=${sha}"
echo "key=overdrive-ch-seed-x86_64-${rev}-${toolchain}-${sha}"
