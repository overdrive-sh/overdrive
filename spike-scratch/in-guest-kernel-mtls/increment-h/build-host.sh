#!/usr/bin/env bash
# Spike C re-run (GH #303, increment-h): build the host-side rustls programs.
# These are a COPY of increment-f's host sources with ONE deliberate change for
# this re-run (Change 1): the peer's TLS listeners send ZERO TLS 1.3 session
# tickets on every clean mesh port (production-faithful), with one contrast port
# (:6452) keeping rustls' default of 2. The relay's server side already sends 0.
# Target dir + CARGO_HOME + certs all in the unsynced scratch tree. No sudo, no
# system packages. The SVID keys live only in the relay process.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOSTSRC="$INC/host"                                               # increment-h local host sources (modified peer.rs)
export CARGO_TARGET_DIR="$HOME/igkm-spike-a/igkmh-host-target"    # separate from increment-f's target
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="$HOME/igkm-spike-a/cargo-home"
export PATH="$HOME/.cargo/bin:$PATH"
CERTS="$HOME/igkm-spike-a/igkmh/out/certs"
mkdir -p "$CARGO_HOME" "$CERTS"

echo "=== increment-h host sources (peer.rs = production-faithful tickets=0) ==="
sha256sum "$HOSTSRC"/src/lib.rs "$HOSTSRC"/src/bin/*.rs
cd "$HOSTSRC"
rustc --version; cargo --version
echo "=== cargo build --release --locked ==="
cargo build --release --locked 2>&1 | tail -8
for b in gencerts peer relay inclient; do sha256sum "$CARGO_TARGET_DIR/release/$b"; done

echo "=== test PKI -> $CERTS (IP SANs 192.168.204.1/.2; regenerated only if absent) ==="
if [[ -f "$CERTS/client.key" && -f "$CERTS/peer-allowed.key" && -f "$CERTS/guest-server.key" ]]; then
  echo "exists; not regenerated"
else
  rm -rf -- "$CERTS"; mkdir -p "$CERTS"
  "$CARGO_TARGET_DIR/release/gencerts" "$CERTS"
fi
chmod 0600 "$CERTS"/*.key 2>/dev/null || true
ls "$CERTS"
