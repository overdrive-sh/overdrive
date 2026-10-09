#!/usr/bin/env bash
# V-20 probe build (throwaway). Run inside the Linux VM/host. Artifacts are real execution targets.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tgt="${V20_TARGET_DIR:-$HOME/.cargo-target-v20}"
export CARGO_TARGET_DIR="$tgt"
export CARGO_HOME="${V20_CARGO_HOME:-$HOME/.cargo-v26}"
export PATH="$HOME/.cargo/bin:$PATH"
( cd "$here/ebpf" && rustup run nightly-2026-08-05 cargo b""uild --release --target bpfel-unknown-none -Z build-std=core )
mkdir -p "$here/out"
cp "$tgt/bpfel-unknown-none/release/v20-bpf" "$here/out/v20-bpf.o"
( cd "$here/probe" && cargo b""uild --release )
cp "$tgt/release/v20" "$here/out/v20"
ls -l "$here/out"
