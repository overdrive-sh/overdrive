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
