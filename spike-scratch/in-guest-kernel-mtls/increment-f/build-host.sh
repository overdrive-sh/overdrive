#!/usr/bin/env bash
# Spike B3 host build (GH #303, increment-f; metal login user, NO sudo, NO system packages).
#
# Builds the host programs (gencerts, peer, relay, inclient) from ./host with
# `cargo build --release --locked` against the committed Cargo.lock, into a
# target dir OUTSIDE the rsynced tree, then generates the throwaway test PKI
# into the gitignored ./out/certs (kept across runs; regenerated only if absent).
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export CARGO_TARGET_DIR="$HOME/igkm-spike-a/igkmf-host-target"
readonly CERTS="$INCREMENT/out/certs"

step() { printf '\n##### [build-host] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

step "toolchain"
# Run 0002: the shared ~/.cargo/registry has root-owned entries, so the login
# user cannot write to it ("Permission denied" on the crate cache). Leave it
# alone and use a spike-private CARGO_HOME in the scratch tree instead; the
# rustup toolchain (RUSTUP_HOME) is shared read-only.
echo "root-owned entries in the shared ~/.cargo/registry (first 3): $(find "$HOME/.cargo/registry" -maxdepth 3 -user root 2>/dev/null | head -3 | tr '\n' ' ')"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="$HOME/igkm-spike-a/cargo-home"
mkdir -p "$CARGO_HOME"
export PATH="$HOME/.cargo/bin:$PATH"
echo "CARGO_HOME=$CARGO_HOME RUSTUP_HOME=$RUSTUP_HOME"
cd "$INCREMENT/host"
rustc --version
cargo --version
sha256sum Cargo.toml Cargo.lock src/lib.rs src/bin/*.rs

step "pinned crate versions (Cargo.lock)"
python3 - <<'PY'
import re
lock = open("Cargo.lock").read()
for pkg in re.findall(r'\[\[package\]\]\nname = "([^"]+)"\nversion = "([^"]+)"', lock):
    if pkg[0] in ("rustls", "rustls-webpki", "rustls-pki-types", "ring", "rcgen", "x509-parser", "time"):
        print(f"  {pkg[0]} {pkg[1]}")
PY

step "cargo build --release --locked"
cargo build --release --locked 2>&1
for b in gencerts peer relay inclient; do
  sha256sum "$CARGO_TARGET_DIR/release/$b"
done

step "test PKI -> $CERTS"
if [[ -f "$CERTS/client.key" && -f "$CERTS/guest-server.key" && -f "$CERTS/peer-client-denied.key" ]]; then
  echo "exists; not regenerated"
else
  rm -rf -- "$CERTS"
  "$CARGO_TARGET_DIR/release/gencerts" "$CERTS"
fi
chmod 0600 "$CERTS"/*.key
ls -l "$CERTS"
for c in ca client peer-allowed peer-denied guest-server peer-client peer-client-denied; do
  echo "--- $c.pem"
  openssl x509 -in "$CERTS/$c.pem" -noout -subject -issuer -dates -ext subjectAltName,extendedKeyUsage 2>&1
  openssl x509 -in "$CERTS/$c.pem" -noout -fingerprint -sha256
done
echo "--- chain check"
for c in client peer-allowed peer-denied guest-server peer-client peer-client-denied; do
  openssl verify -CAfile "$CERTS/ca.pem" -attime "$(date -d 2026-10-01 +%s)" "$CERTS/$c.pem"
done
for k in client guest-server; do
  echo "--- $k key (held by the relay only): type and sha256 of the PKCS#8 DER"
  openssl pkey -in "$CERTS/$k.key" -noout -text 2>/dev/null | head -1
  openssl pkey -in "$CERTS/$k.key" -outform DER | sha256sum
done
