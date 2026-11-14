#!/usr/bin/env bash
# Build the pinned Cloud Hypervisor fork exactly as the user provision block of
# infra/lima/overdrive-dev.yaml does (x86_64 musl, --features mshv, --locked,
# pinned toolchain, metadata-free source archive of the pinned revision), and
# stage the checksum-verified binary at $SEED_OUT for Lima provisioning to
# consume. Inputs (from .github/scripts/ch-pins.sh): CH_REPO CH_REV
# CH_TOOLCHAIN CH_EXPECTED, plus SEED_OUT.
#
# The binary embeds absolute source and cargo-registry paths, so the pinned
# checksum is only reproducible from the same paths it was pinned under:
# source at /var/tmp/overdrive/cloud-hypervisor/<rev>/source and CARGO_HOME at
# /home/ubuntu/.cargo (validated on the native metal host: a different source
# path or CARGO_HOME yields a different sha256; a different target dir does
# not). Hence the fixed paths below.
set -euxo pipefail

CH_TARGET="x86_64-unknown-linux-musl"
CH_ROOT="/var/tmp/overdrive/cloud-hypervisor/${CH_REV}"
CH_SOURCE="${CH_ROOT}/source"
CH_BUILD="${CH_ROOT}/build"
CH_BUILD_BIN="${CH_BUILD}/${CH_TARGET}/release/cloud-hypervisor"

sudo mkdir -p "${CH_ROOT}" /home/ubuntu
sudo chown -R "$(id -u):$(id -g)" "${CH_ROOT}" /home/ubuntu
export CARGO_HOME=/home/ubuntu/.cargo
export RUSTUP_HOME="${RUSTUP_HOME:-${HOME}/.rustup}"
export PATH="${HOME}/.cargo/bin:${CARGO_HOME}/bin:${PATH}"

rustup toolchain install "${CH_TOOLCHAIN}" --profile minimal
rustup target add --toolchain "${CH_TOOLCHAIN}" "${CH_TARGET}"
command -v musl-gcc >/dev/null 2>&1 || { echo "FATAL: musl-gcc is required for ${CH_TARGET}" >&2; exit 1; }
command -v strip >/dev/null 2>&1 || { echo "FATAL: native strip is required for ${CH_TARGET}" >&2; exit 1; }

CH_FETCH="$(mktemp -d "${CH_ROOT}/.fetch.XXXXXX")"
CH_SOURCE_TMP="$(mktemp -d "${CH_ROOT}/.source.XXXXXX")"
git -C "${CH_FETCH}" init --quiet
git -C "${CH_FETCH}" remote add origin "https://github.com/${CH_REPO}.git"
git -C "${CH_FETCH}" fetch --depth=1 origin "${CH_REV}"
CH_FETCHED_REV="$(git -C "${CH_FETCH}" rev-parse FETCH_HEAD)"
if [ "${CH_FETCHED_REV}" != "${CH_REV}" ]; then
  echo "FATAL: fetched Cloud Hypervisor source ${CH_FETCHED_REV}, expected ${CH_REV}" >&2
  exit 1
fi
git -C "${CH_FETCH}" archive --format=tar "${CH_REV}" | tar -xf - -C "${CH_SOURCE_TMP}"
[ ! -e "${CH_SOURCE_TMP}/.git" ] || { echo "FATAL: source archive contains .git" >&2; exit 1; }
rm -rf "${CH_SOURCE}"
mv "${CH_SOURCE_TMP}" "${CH_SOURCE}"
rm -rf "${CH_FETCH}"

CARGO_TARGET_DIR="${CH_BUILD}" cargo "+${CH_TOOLCHAIN}" build \
  --manifest-path "${CH_SOURCE}/Cargo.toml" \
  --all --release --features mshv --target "${CH_TARGET}" --locked
[ -f "${CH_BUILD_BIN}" ] || { echo "FATAL: build artifact missing: ${CH_BUILD_BIN}" >&2; exit 1; }
strip "${CH_BUILD_BIN}"
if ! printf '%s  %s\n' "${CH_EXPECTED}" "${CH_BUILD_BIN}" | sha256sum --check --status; then
  echo "FATAL: built Cloud Hypervisor checksum mismatch (got $(sha256sum "${CH_BUILD_BIN}")), expected ${CH_EXPECTED}" >&2
  exit 1
fi
mkdir -p "$(dirname "${SEED_OUT}")"
install -m 0755 "${CH_BUILD_BIN}" "${SEED_OUT}"
printf '%s  %s\n' "${CH_EXPECTED}" "${SEED_OUT}" | sha256sum --check
