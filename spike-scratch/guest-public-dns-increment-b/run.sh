#!/usr/bin/env bash
# Run under: cargo xtask metal run --exclusive -- spike-scratch/guest-public-dns-increment-b/run.sh <out-subdir>
set -uo pipefail
umask 022
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/out/${1:?out subdir}"
mkdir -p "$out"
exec > >(tee -a "$out/run.log") 2>&1
echo "uname: $(uname -r)  sha: $(git -C "$here" rev-parse HEAD)  dirty:"; git -C "$here" status --short | head -20
echo "glibc: $(ldd --version | head -1)"
echo "--- forwarder module before:"; lsmod | grep -i -E "ovd|overdrive" || echo "(none)"
( cd "$here/../.." && cargo xtask kmod-load ) || exit 11
rustc --edition 2021 --target x86_64-unknown-linux-musl -O "$here/guest/probe.rs" -o "$out/dns-probe" || exit 10
rustc --edition 2021 -O "$here/guest/probe.rs" -o "$out/dns-probe-glibc" || exit 12
file "$out/dns-probe" "$out/dns-probe-glibc" 2>/dev/null; ldd "$out/dns-probe-glibc"
export CARGO_TARGET_DIR="$(cd "$here/../.." && pwd)/target"
cd "$(cd "$here/../.." && pwd)" && cargo run --manifest-path "$here/Cargo.toml" --bin harness -- "$out/dns-probe" "$out" "$out/dns-probe-glibc"
echo "harness exit=$?"
