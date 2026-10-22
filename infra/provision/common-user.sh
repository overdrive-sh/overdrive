#!/usr/bin/env bash
# User-mode provisioning for the BARE-METAL box, as the UNPRIVILEGED build
# user (never root). Written to be reusable by Lima, but Lima does not
# currently invoke it — see the scope note in common-system.sh.
#
# Idempotent: safe to re-run.
#
# Usage:  infra/provision/common-user.sh
set -euo pipefail

log() { printf '\n=== [common-user] %s\n' "$*"; }

[ "$(id -u)" -ne 0 ] || { echo "must NOT run as root — rustup belongs to the build user" >&2; exit 1; }

# ---------------------------------------------------------------------------
# CWD MUST BE THE REPO. Everything below depends on `rust-toolchain.toml` being
# resolvable, and `--default-toolchain none` means rustup has NO fallback.
#
# ssh lands in $HOME, not the repo. Running from $HOME, every rustup/cargo
# invocation below fails with:
#   error: rustup could not choose a version of rustup to run, because one
#          wasn't specified explicitly, and no default is configured
# — which under `set -e` kills the script at the LAST step of an otherwise
# successful bootstrap. Verified 2026-08-10.
# ---------------------------------------------------------------------------
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${REPO}"
[ -f rust-toolchain.toml ] || { echo "FATAL: no rust-toolchain.toml at ${REPO}" >&2; exit 1; }
set -a
# shellcheck disable=SC1091
. "${REPO}/infra/provision/versions.env"
set +a

log "rustup (repo: ${REPO})"
if ! command -v rustup >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --default-toolchain none --profile minimal
fi
# shellcheck disable=SC1091
. "${HOME}/.cargo/env"

# Materialise the pinned toolchain BEFORE anything else touches cargo/rustup.
# `rust-toolchain.toml` already declares the components, so no separate
# `component add` is needed — and the old `|| true` on that line was hiding
# this exact failure.
rustup toolchain install
rustc --version

# ---------------------------------------------------------------------------
# The musl target for statically-linked guest binaries.
#
# ARCH-SWITCHED ON PURPOSE. The spike probes build a static guest init, and the
# triple follows the HOST arch — aarch64 in the Apple-Silicon Lima VM, x86_64
# on the metal box. Hardcoding either one silently breaks the other, and the
# failure surfaces as a confusing link error rather than "wrong target".
# ---------------------------------------------------------------------------
MUSL_TARGET="$(uname -m)-unknown-linux-musl"
log "rust target ${MUSL_TARGET}"
rustup target add "${MUSL_TARGET}"

# ---------------------------------------------------------------------------
# Cloud Hypervisor — build the pinned fork locally, stage verified bytes, then
# install only the architecture-selected artifact. No release binary fallback.
# ---------------------------------------------------------------------------
provision_cloud_hypervisor() {
  local arch target expected
  local root source_dir build_dir artifact build_binary installed remote fetch_dir source_tmp fetched_rev
  local -a build_args

  arch="$(uname -m)"
  case "${arch}" in
    x86_64)
      target="x86_64-unknown-linux-musl"
      expected="${CLOUD_HYPERVISOR_SHA256_X86_64}"
      build_args=(--all --release --features mshv --target "${target}" --locked)
      ;;
    aarch64)
      target="aarch64-unknown-linux-musl"
      expected="${CLOUD_HYPERVISOR_SHA256_AARCH64}"
      build_args=(--all --release --target "${target}" --locked)
      ;;
    *)
      echo "FATAL: unsupported Cloud Hypervisor build architecture: ${arch}" >&2
      return 1
      ;;
  esac

  root="/var/tmp/overdrive/cloud-hypervisor/${CLOUD_HYPERVISOR_FORK_REV}"
  source_dir="${root}/source"
  build_dir="${root}/build"
  artifact="${root}/artifacts/${target}/cloud-hypervisor"
  build_binary="${build_dir}/${target}/release/cloud-hypervisor"
  installed="/usr/local/bin/cloud-hypervisor"
  remote="https://github.com/${CLOUD_HYPERVISOR_FORK_REPO}.git"

  log "cloud-hypervisor ${CLOUD_HYPERVISOR_FORK_REPO}@${CLOUD_HYPERVISOR_FORK_REV} (${target})"
  rustup toolchain install "${CLOUD_HYPERVISOR_BUILD_TOOLCHAIN}" --profile minimal
  rustup target add --toolchain "${CLOUD_HYPERVISOR_BUILD_TOOLCHAIN}" "${target}"
  command -v musl-gcc >/dev/null 2>&1 || {
    echo "FATAL: musl-gcc is required for ${target}" >&2
    return 1
  }
  command -v strip >/dev/null 2>&1 || {
    echo "FATAL: native strip is required for ${target}" >&2
    return 1
  }

  mkdir -p "${root}"
  if [ -e "${source_dir}/.git" ]; then
    echo "FATAL: Cloud Hypervisor source must be metadata-free: ${source_dir}" >&2
    return 1
  fi
  if [ ! -f "${source_dir}/Cargo.toml" ]; then
    if [ -e "${source_dir}" ]; then
      echo "FATAL: incomplete local Cloud Hypervisor source: ${source_dir}" >&2
      return 1
    fi
    fetch_dir="$(mktemp -d "${root}/.fetch.XXXXXX")"
    source_tmp="$(mktemp -d "${root}/.source.XXXXXX")"
    git -C "${fetch_dir}" init --quiet
    git -C "${fetch_dir}" remote add origin "${remote}"
    git -C "${fetch_dir}" fetch --depth=1 origin "${CLOUD_HYPERVISOR_FORK_REV}"
    fetched_rev="$(git -C "${fetch_dir}" rev-parse FETCH_HEAD)"
    if [ "${fetched_rev}" != "${CLOUD_HYPERVISOR_FORK_REV}" ]; then
      echo "FATAL: fetched Cloud Hypervisor source ${fetched_rev}, expected ${CLOUD_HYPERVISOR_FORK_REV}" >&2
      return 1
    fi
    git -C "${fetch_dir}" archive --format=tar "${CLOUD_HYPERVISOR_FORK_REV}" \
      | tar -xf - -C "${source_tmp}"
    [ ! -e "${source_tmp}/.git" ] || {
      echo "FATAL: metadata-free Cloud Hypervisor source contains .git" >&2
      return 1
    }
    mv "${source_tmp}" "${source_dir}"
    rm -rf "${fetch_dir}"
  fi
  [ -f "${source_dir}/Cargo.toml" ] || {
    echo "FATAL: pinned Cloud Hypervisor source missing at ${source_dir}" >&2
    return 1
  }

  if [ -e "${artifact}" ]; then
    [ -f "${artifact}" ] || {
      echo "FATAL: staged Cloud Hypervisor artifact is not a file: ${artifact}" >&2
      return 1
    }
    if ! printf '%s  %s\n' "${expected}" "${artifact}" | sha256sum --check --status; then
      echo "FATAL: staged Cloud Hypervisor checksum mismatch: ${artifact}" >&2
      return 1
    fi
  else
    log "building local fork with Rust ${CLOUD_HYPERVISOR_BUILD_TOOLCHAIN}"
    CARGO_TARGET_DIR="${build_dir}" cargo "+${CLOUD_HYPERVISOR_BUILD_TOOLCHAIN}" build \
      --manifest-path "${source_dir}/Cargo.toml" "${build_args[@]}"
    [ -f "${build_binary}" ] || {
      echo "FATAL: Cloud Hypervisor build artifact missing: ${build_binary}" >&2
      return 1
    }
    strip "${build_binary}"
    if ! printf '%s  %s\n' "${expected}" "${build_binary}" | sha256sum --check --status; then
      echo "FATAL: built Cloud Hypervisor checksum mismatch; refusing to stage or install" >&2
      return 1
    fi
    mkdir -p "$(dirname "${artifact}")"
    install -m 0755 "${build_binary}" "${artifact}"
    if ! printf '%s  %s\n' "${expected}" "${artifact}" | sha256sum --check --status; then
      echo "FATAL: staged Cloud Hypervisor checksum mismatch: ${artifact}" >&2
      return 1
    fi
  fi

  if [ ! -f "${installed}" ] || [ ! -x "${installed}" ] \
     || ! printf '%s  %s\n' "${expected}" "${installed}" | sha256sum --check --status; then
    log "installing checksum-verified Cloud Hypervisor at ${installed}"
    sudo -n install -m 0755 "${artifact}" "${installed}"
  else
    log "installed Cloud Hypervisor already matches the pinned checksum"
  fi
  if ! printf '%s  %s\n' "${expected}" "${installed}" | sha256sum --check --status; then
    echo "FATAL: installed Cloud Hypervisor checksum mismatch: ${installed}" >&2
    return 1
  fi
  "${installed}" --version
  if ! "${installed}" --help 2>&1 | grep -q -- '--landlock'; then
    echo "FATAL: this Cloud Hypervisor build has no --landlock" >&2
    return 1
  fi
}

provision_cloud_hypervisor

# ---------------------------------------------------------------------------
# Cargo tooling. binstall pulls prebuilts where crates publish them; a cold
# `cargo install --locked` of these takes many minutes on a 4-core box.
# ---------------------------------------------------------------------------
log "cargo tooling"
if ! command -v cargo-binstall >/dev/null 2>&1; then
  curl -fsSL https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash
fi
cargo binstall --no-confirm --force \
  cargo-deny cargo-nextest cargo-mutants bpf-linker sccache || true

# VERIFY rather than trust the `|| true` above. Without this the box can finish
# "successfully" with no nextest and no musl target, and the first probe run
# fails for a reason that looks nothing like provisioning.
log "verifying"
MISSING=""
rustup target list --installed | grep -qx "${MUSL_TARGET}" || MISSING="${MISSING} target:${MUSL_TARGET}"
for t in cargo-nextest cargo-mutants; do
  command -v "$t" >/dev/null 2>&1 || MISSING="${MISSING} ${t}"
done
if [ -n "${MISSING}" ]; then
  echo "FATAL: provisioning incomplete —${MISSING}" >&2
  exit 1
fi

log "done — $(rustc --version), target ${MUSL_TARGET}"
