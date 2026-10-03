#!/usr/bin/env bash
# Spike D (GH #303, increment-i): build the host-side rustls programs. These are
# a COPY of Spike C's host sources with the Spike D changes: the relay gains a
# service registry + first-healthy-by-Ord resolution (ADR-0072 MtlsResolve) on
# the SAME vsock channel, and the peer stands up two labelled backends for one
# service. Production-faithful TLS (tickets=0, no KeyUpdate) throughout.
# Target dir + certs in the unsynced scratch tree; CARGO_HOME reused from the
# Spike C scratch. No sudo, no system packages. SVID keys live only in the relay.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOSTSRC="$INC/host"                                               # increment-i local host sources
export CARGO_TARGET_DIR="$HOME/igkm-spike-a/igkmd-host-target"    # separate Spike D target
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"
export CARGO_HOME="$HOME/igkm-spike-a/cargo-home"                 # reuse Spike C crate cache
export PATH="$HOME/.cargo/bin:$PATH"
CERTS="$HOME/igkm-spike-a/igkmd/out/certs"
mkdir -p "$CARGO_HOME" "$CERTS"

echo "=== increment-i host sources (relay resolve/LB + two labelled backends) ==="
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
