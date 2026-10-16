#!/bin/bash
# bpf-credit probe: compile on the metal host as the login user, in /tmp/bpf-credit-work only (private; nothing shared is touched).
set -euo pipefail
W=/tmp/bpf-credit-work
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
export CARGO_HOME=/tmp/v3-work/out/cargo-home   # offline registry cache from the V-3 build
cp -f /tmp/v3-work/out/cloud-hypervisor $W/out/cloud-hypervisor
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/bpf/Cargo.toml > $W/out/bpf-build.log 2>&1 || { cat $W/out/bpf-build.log; exit 1; }
cp -f $W/out/bpf-target/bpfel-unknown-none/release/valve-bpf $W/out/valve-bpf.o
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml > $W/out/probe-build.log 2>&1 || { cat $W/out/probe-build.log; exit 1; }
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/valveprobe $W/out/valveprobe
CARGO_TARGET_DIR=$W/out/elig-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/src/elig/Cargo.toml > $W/out/elig-build.log 2>&1 || { cat $W/out/elig-build.log; exit 1; }
cp -f $W/out/elig-target/x86_64-unknown-linux-gnu/release/eligprobe $W/out/eligprobe
grep -c warning $W/out/probe-build.log || true
sha256sum $W/out/valve-bpf.o $W/out/valveprobe $W/out/eligprobe $W/out/cloud-hypervisor
