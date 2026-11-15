#!/usr/bin/env bash
# Run under: cargo xtask metal run --exclusive -- env KEY=VAL ... spike-scratch/guest-udp-vip-probe/run.sh <out-subdir>
set -uo pipefail
umask 022
here="$(cd "$(dirname "$0")" && pwd)"
out="$here/out/${1:?out subdir}"
mkdir -p "$out"
exec > >(tee -a "$out/run.log") 2>&1
echo "uname: $(uname -r)  sha: $(git -C "$here" rev-parse HEAD)  dirty:"; git -C "$here" status --short | head
rustc --edition 2021 --target x86_64-unknown-linux-musl -O "$here/guest/probe.rs" -o "$out/udp-probe" || exit 10
export CARGO_TARGET_DIR="$(cd "$here/../.." && pwd)/target"
cd "$(cd "$here/../.." && pwd)" && cargo run --manifest-path "$here/Cargo.toml" --bin harness -- "$out/udp-probe" "$out"
echo "harness exit=$?"
