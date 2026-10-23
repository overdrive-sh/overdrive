#!/usr/bin/env bash
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
target="${TCP_CORRELATION_TARGET_DIR:-/tmp/tcp-correlation-target}"
cg="/sys/fs/cgroup/tcp-correlation-option-$$"
cleanup() {
  rmdir "$cg" 2>/dev/null || true
}
trap cleanup EXIT
mkdir "$cg"
export CARGO_TARGET_DIR="$target"
export RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3"
(cd "$here/bpf" && cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core)
unset RUSTFLAGS
(cd "$here/probe" && cargo build --release)
"$target/release/tcp-correlation-option-probe" \
  "$target/bpfel-unknown-none/release/tcp-correlation-option-bpf" "$cg"
