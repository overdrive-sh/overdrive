#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="${TCP_CORRELATION_SCOPE_VALIDATION_CH_TARGET_DIR:-/tmp/tcp-correlation-scope-validation-ch-target}"
export CARGO_TARGET_DIR="$target"
export RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3"
(cd "$here/bpf" && cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core)
unset RUSTFLAGS

probe_target="x86_64-unknown-linux-musl"
CC_x86_64_unknown_linux_musl=musl-gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    cargo build --manifest-path "$here/probe/Cargo.toml" --release --target "$probe_target"

cargo xtask guest-kit-prepare
cargo run --manifest-path "$here/runner/Cargo.toml" --release -- \
    "$target/$probe_target/release/tcp-correlation-ingress-probe" \
    "$target/bpfel-unknown-none/release/tcp-correlation-ingress-bpf"
