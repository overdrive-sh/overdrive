#!/usr/bin/env bash
# Cross-build the x86_64 probe (static musl) from the aarch64 Lima VM; the BPF object is arch-independent.
# The artifact is a real execution target for the metal host, hence cargo build.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
export CARGO_TARGET_DIR="${V17_TARGET_DIR:-$HOME/.cargo-target-v17}"
export CARGO_HOME="${V17_CARGO_HOME:-$HOME/.cargo-v17}"
export PATH="$HOME/.cargo/bin:$PATH"
rustup target list --installed
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
( cd "$here/probe" && cargo build --release --target x86_64-unknown-linux-musl )
cp "$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/v17" "$here/out/v17-x86_64"
file "$here/out/v17-x86_64"
