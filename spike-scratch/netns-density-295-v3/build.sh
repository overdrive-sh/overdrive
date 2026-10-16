#!/bin/bash
# V-3 probe: compile on the metal host as the login user, in /tmp/v3-work only (private; nothing shared is touched).
set -euo pipefail
W=/tmp/v3-work
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
SRC=/home/ubuntu/overdrive/spike-scratch
[ -d $W/out/cargo-home ] || cp -r $SRC/netns-density-295-guest-vsock-capture/out/cargo-home $W/out/cargo-home
export CARGO_HOME=$W/out/cargo-home
cp -f $SRC/netns-density-295-v24/out/ch-target/release/cloud-hypervisor $W/out/cloud-hypervisor
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/bpf/Cargo.toml 2>&1 | tail -40
cp -f $W/out/bpf-target/bpfel-unknown-none/release/v3-bpf $W/out/v3-bpf.o
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml 2>&1 | tail -60
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/v3probe $W/out/v3probe
sha256sum $W/out/v3-bpf.o $W/out/v3probe $W/out/cloud-hypervisor
