#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="${TCP_CORRELATION_TRACE_VALIDATION_CH_TARGET_DIR:-/tmp/tcp-correlation-trace-validation-ch-target}"
export CARGO_TARGET_DIR="$target"
export RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3"
(cd "$here/trace-base-bpf" && cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core)
unset RUSTFLAGS

core_dir="$target/core-trace"
mkdir -p "$core_dir"
bpftool btf dump file /sys/kernel/btf/vmlinux format c > "$core_dir/vmlinux.h"
clang -g -O2 -target bpf -D__TARGET_ARCH_x86 \
    -I "$core_dir" -I /usr/include/x86_64-linux-gnu \
    -c "$here/core-trace/trace.bpf.c" -o "$core_dir/trace.bpf.o"

probe_target="x86_64-unknown-linux-musl"
CC_x86_64_unknown_linux_musl=musl-gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=musl-gcc \
    cargo build --manifest-path "$here/trace-probe/Cargo.toml" --release --target "$probe_target"

cargo xtask guest-kit-prepare
cargo run --manifest-path "$here/trace-runner/Cargo.toml" --release -- \
    "$target/$probe_target/release/tcp-correlation-ingress-probe" \
    "$target/bpfel-unknown-none/release/tcp-correlation-trace-base-bpf" \
    "$core_dir/trace.bpf.o"
