#!/usr/bin/env bash
# Install the pinned Quint formal-methods toolchain: a pinned Node.js LTS with
# the `@informalsystems/quint` npm package installed globally, a Temurin JDK 21 (Apalache and TLC run on the JVM), and a
# pre-populated QUINT_HOME holding the Apalache distribution and the Rust
# evaluator that Quint would otherwise download on first use.
#
# CONSUMED BY: the `quint` job in .github/workflows/ci.yml.
# infra/lima/overdrive-dev.yaml carries the same pins inline in its
# `param:` block and its Quint provisioning step (Lima templates cannot
# reference a file in a particular checkout); keep the two aligned.
#
# Usage: install-quint.sh <prefix> [<bin-dir>]
#
#   <prefix>/node                          Node.js LTS (official tarball)
#   <prefix>/npm                           npm global prefix holding quint
#   <prefix>/jdk                           Temurin JDK 21
#   <prefix>/quint-home                    QUINT_HOME (Apalache + evaluator)
#   <bin-dir>/quint                        wrapper that defaults QUINT_HOME and
#                                          JAVA_HOME to the paths above, so the
#                                          toolchain works under `sudo env PATH=…`
#                                          without any further environment
#   <bin-dir>/java                         symlink to the JDK's java
#
# <bin-dir> defaults to <prefix>/bin. Idempotent: every artifact is
# checksum-verified on download and skipped when already present.
set -euo pipefail

# Why npm and not the release binary: the Quint 0.32 release binary is a
# `deno compile` build whose HTTP/2 client intermittently stalls talking to
# the Apalache server (reply stuck at the 64 KiB flow-control window). The
# same version under Node.js does not use that client.
#
# Pins. Quint v0.33.0 exists (2026-09-28); v0.32.0 is held because it is
# the version the quint-connect 0.1.2 conformance precedent was validated
# against (Apalache 0.56.1, evaluator v0.6.0). Clears when v0.33.0 is run
# against the specs/quint/** checks and quint-connect.
QUINT_VERSION="0.32.0"
# sha512 of the registry tarball quint-${QUINT_VERSION}.tgz (npm `dist.integrity`).
QUINT_TGZ_SHA512="f58a238bb0d4ea6a119b5a1f0da37e7ed77135c1067228866e6ede37832395c09b7efeff63fcdc85a75c5d746631998df684cf5d493d1a686340e8400c9b2249"
# Node.js 24 LTS ("Krypton"); latest 24.x as of 2026-10. Tarball sha256 from
# https://nodejs.org/dist/v24.21.0/SHASUMS256.txt.
NODE_VERSION="v24.21.0"
NODE_SHA256_X64="fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6"
NODE_SHA256_ARM64="6ad1325edbdb5649c379b75a237147a666c95d4f9ae8d340fef2d1575d289ad2"
# Apalache version Quint v0.32.0 downloads by default (`quint verify
# --apalache-version` default); used to verify the pre-fetch landed.
APALACHE_VERSION="0.56.1"
JDK_RELEASE="jdk-21.0.12.1+1"
JDK_FILE_VERSION="21.0.12.1_1"
JDK_SHA256_X64="ce79869e1307ed8ee1e2baa86a412b1eb5b75d10a01006d788a6f968bcfaee94"
JDK_SHA256_AARCH64="23e37e026f12f3e706f18938ff611db3032d075b09d0879a25d06718c773e223"

PREFIX="${1:?usage: install-quint.sh <prefix> [<bin-dir>]}"
BIN_DIR="${2:-${PREFIX}/bin}"

case "$(uname -m)" in
  x86_64)
    NARCH=x64; NSHA="${NODE_SHA256_X64}"
    JARCH=x64; JSHA="${JDK_SHA256_X64}" ;;
  aarch64 | arm64)
    NARCH=arm64; NSHA="${NODE_SHA256_ARM64}"
    JARCH=aarch64; JSHA="${JDK_SHA256_AARCH64}" ;;
  *) echo "install-quint: unsupported arch $(uname -m)" >&2; exit 1 ;;
esac

NODE_DIR="${PREFIX}/node"
NPM_PREFIX="${PREFIX}/npm"
JDK_DIR="${PREFIX}/jdk"
QUINT_HOME_DIR="${PREFIX}/quint-home"
WORK="$(mktemp -d)"
trap 'rm -rf "${WORK}"' EXIT

mkdir -p "${QUINT_HOME_DIR}" "${NPM_PREFIX}" "${BIN_DIR}"

# --- Node.js LTS --------------------------------------------------------------
if [ ! -x "${NODE_DIR}/bin/node" ]; then
  curl -fsSL -o "${WORK}/node.tar.xz" \
    "https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-linux-${NARCH}.tar.xz"
  echo "${NSHA}  ${WORK}/node.tar.xz" | sha256sum -c -
  rm -rf "${NODE_DIR}"
  mkdir -p "${NODE_DIR}"
  tar -xJf "${WORK}/node.tar.xz" -C "${NODE_DIR}" --strip-components=1
fi

# --- Quint (npm package, global) ----------------------------------------------
if [ ! -x "${NPM_PREFIX}/bin/quint" ] \
   || [ "$("${NODE_DIR}/bin/node" "${NPM_PREFIX}/bin/quint" --version)" != "${QUINT_VERSION}" ]; then
  curl -fsSL -o "${WORK}/quint.tgz" \
    "https://registry.npmjs.org/@informalsystems/quint/-/quint-${QUINT_VERSION}.tgz"
  echo "${QUINT_TGZ_SHA512}  ${WORK}/quint.tgz" | sha512sum -c -
  PATH="${NODE_DIR}/bin:${PATH}" npm install -g --prefix "${NPM_PREFIX}" "${WORK}/quint.tgz"
fi

# --- Temurin JDK 21 ---------------------------------------------------------
if [ ! -x "${JDK_DIR}/bin/java" ]; then
  JDK_TGZ="OpenJDK21U-jdk_${JARCH}_linux_hotspot_${JDK_FILE_VERSION}.tar.gz"
  curl -fsSL -o "${WORK}/jdk.tar.gz" \
    "https://github.com/adoptium/temurin21-binaries/releases/download/${JDK_RELEASE/+/%2B}/${JDK_TGZ}"
  echo "${JSHA}  ${WORK}/jdk.tar.gz" | sha256sum -c -
  rm -rf "${JDK_DIR}"
  mkdir -p "${JDK_DIR}"
  # `legal/` holds only license texts stored as hard links, which virtiofs
  # mounts refuse — skip it.
  tar -xzf "${WORK}/jdk.tar.gz" -C "${JDK_DIR}" --strip-components=1 --exclude='*/legal'
fi

# --- Wrapper + java on PATH -------------------------------------------------
cat > "${BIN_DIR}/quint" <<EOF
#!/bin/sh
# Generated by infra/provision/install-quint.sh — Quint ${QUINT_VERSION}.
export QUINT_HOME="\${QUINT_HOME:-${QUINT_HOME_DIR}}"
export JAVA_HOME="\${JAVA_HOME:-${JDK_DIR}}"
export PATH="${NODE_DIR}/bin:\${JAVA_HOME}/bin:\${PATH}"
exec "${NPM_PREFIX}/bin/quint" "\$@"
EOF
chmod 0755 "${BIN_DIR}/quint"
ln -sf "${JDK_DIR}/bin/java" "${BIN_DIR}/java"

# --- Pre-fetch Apalache + the Rust evaluator into QUINT_HOME ----------------
# Quint downloads both lazily on first `verify` / `run`. Trigger the
# downloads now, against a trivial bounded spec, so check runs never touch
# the network. A dedicated server port keeps this from attaching to any
# Apalache server another process already runs on Quint's default port.
if [ ! -x "${QUINT_HOME_DIR}/apalache-dist-${APALACHE_VERSION}/apalache/bin/apalache-mc" ] \
   || ! ls -d "${QUINT_HOME_DIR}"/rust-evaluator-* >/dev/null 2>&1; then
  cat > "${WORK}/prefetch.qnt" <<'EOF'
module prefetch {
  var x: int
  action init = x' = 0
  action step = all { x < 2, x' = x + 1 }
  val ok = x >= 0
}
EOF
  (
    cd "${WORK}"
    export TMPDIR="${WORK}"
    QUINT_HOME="${QUINT_HOME_DIR}" "${BIN_DIR}/quint" run prefetch.qnt \
      --invariant=ok --max-samples=1 --max-steps=2 </dev/null
    QUINT_HOME="${QUINT_HOME_DIR}" "${BIN_DIR}/quint" verify prefetch.qnt \
      --invariant=ok --max-steps=2 --server-endpoint=localhost:18821 </dev/null
  )
fi
test -x "${QUINT_HOME_DIR}/apalache-dist-${APALACHE_VERSION}/apalache/bin/apalache-mc"
ls -d "${QUINT_HOME_DIR}"/rust-evaluator-* >/dev/null

# Readable by every user (the Lima VM provisions as root; checks run as the
# `lima` user and as root under `cargo xtask lima run`).
chmod -R a+rX "${PREFIX}"

echo "quint $("${BIN_DIR}/quint" --version)"
"${BIN_DIR}/java" -version 2>&1 | head -n1
ls "${QUINT_HOME_DIR}"
