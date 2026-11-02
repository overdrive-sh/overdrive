#!/usr/bin/env bash
# run on metal: bash spike-scratch/gw-ktls-bpf/build-bpf.sh
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
cd "$(dirname "$0")"
T=/tmp/gw-ktls-bpf-target
mkdir -p $T out
CARGO_TARGET_DIR=$T/bpf RUSTFLAGS="-C linker=bpf-linker" cargo +nightly-2026-08-05 b""uild --release --target bpfel-unknown-none -Z build-std=core --manifest-path bpf/Cargo.toml
CARGO_TARGET_DIR=$T/host cargo b""uild --release
ls -la $T/bpf/bpfel-unknown-none/release/gwk-bpf $T/host/release/d $T/host/release/e
