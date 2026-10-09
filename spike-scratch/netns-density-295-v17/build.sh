#!/usr/bin/env bash
# V-26 probe build (throwaway). Run inside the Linux VM/host. The artifacts are
# real execution targets (BPF object + probe binary), hence cargo build.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tgt="${V17_TARGET_DIR:-$HOME/.cargo-target-v17}"
export CARGO_TARGET_DIR="$tgt"
# the shared ~/.cargo registry is root-owned from earlier root runs; use a private one
export CARGO_HOME="${V17_CARGO_HOME:-$HOME/.cargo-v17}"
export PATH="$HOME/.cargo/bin:$PATH"
( cd "$here/ebpf" && rustup run nightly-2026-08-05 cargo build --release --target bpfel-unknown-none -Z build-std=core )
mkdir -p "$here/out"
cp "$tgt/bpfel-unknown-none/release/v17-bpf" "$here/out/v17-bpf.o"
( cd "$here/probe" && cargo build --release )
cp "$tgt/release/v17" "$here/out/v17"
ls -l "$here/out"
