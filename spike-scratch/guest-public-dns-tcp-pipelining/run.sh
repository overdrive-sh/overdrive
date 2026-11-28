#!/usr/bin/env bash
# Run: cargo xtask metal run -- spike-scratch/guest-public-dns-tcp-pipelining/run.sh <out-subdir>
# Compiles the harness + a musl client, then runs everything inside a private net+mount namespace.
set -uo pipefail
umask 022
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../.." && pwd)"
out="$here/out/${1:?out subdir}"
mkdir -p "$out"
exec > >(tee -a "$out/run.log") 2>&1
echo "== env"; uname -a; echo "kernel: $(uname -r)"; ldd --version | head -1; date -u
echo "sha: $(git -C "$root" rev-parse HEAD)"; git -C "$root" status --short | head
for t in tcpdump strace getent unshare ip bc ss; do printf '%s: ' $t; command -v $t || echo MISSING; done
rustup target list --installed 2>/dev/null | grep musl || echo "(musl target not listed by rustup)"
export CARGO_TARGET_DIR="$root/target"
cargo b --manifest-path "$here/Cargo.toml" --bin harness || exit 10
rustc --edition 2021 --target x86_64-unknown-linux-musl -O "$here/client/musl_lookup.rs" -o "$out/musl_lookup" || echo "MUSL BUILD FAILED"
cp "$CARGO_TARGET_DIR/debug/harness" "$out/harness"
exec unshare --net --mount --propagation private bash "$here/inner.sh" "$out" "$here" "${2:-v4}"
