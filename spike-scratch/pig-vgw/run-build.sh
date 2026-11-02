#!/usr/bin/env bash
# Compile vgw-bpf (bpfel-unknown-none, aya-ebpf Rust) and the userspace bins. Run on metal.
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
cd "$(dirname "$0")"
T=$PWD/target
mkdir -p "$T" out
CARGO_TARGET_DIR=$T/bpf-target RUSTFLAGS="-C linker=bpf-linker" cargo +nightly-2026-08-05 \
  build --release --target bpfel-unknown-none -Z build-std=core --manifest-path bpf/Cargo.toml
CARGO_TARGET_DIR=$T/host cargo build --release
ls -la "$T/bpf-target/bpfel-unknown-none/release/vgw-bpf" "$T/host/release/vgw" "$T/host/release/vgwdial"
