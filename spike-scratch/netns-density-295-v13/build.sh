#!/bin/bash
# V-13: build the probe artifacts on the metal host, as the login user, in /tmp/v13-work only
# (private directory; nothing shared is touched; run under nice, outside the lease).
set -euo pipefail
W=/tmp/v13-work
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
SRC=/home/ubuntu/overdrive/spike-scratch
[ -d $W/out/cargo-home ] || cp -r $SRC/netns-density-295-guest-vsock-capture/out/cargo-home $W/out/cargo-home
export CARGO_HOME=$W/out/cargo-home
# CH fork build from the v24 spike (fork 41a619d19), read-only copy
cp -f $SRC/netns-density-295-v24/out/ch-target/release/cloud-hypervisor $W/out/cloud-hypervisor
sha256sum $W/out/cloud-hypervisor; $W/out/cloud-hypervisor --version
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker" nice -n 19 cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/bpf/Cargo.toml 2>&1 | tail -5
CARGO_TARGET_DIR=$W/out/vfwd-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/src/vfwd/Cargo.toml 2>&1 | tail -40
cp -f $W/out/bpf-target/bpfel-unknown-none/release/vfwd-bpf $W/out/vfwd-bpf.o
cp -f $W/out/vfwd-target/x86_64-unknown-linux-gnu/release/vfwd $W/out/vfwd
sha256sum $W/out/vfwd $W/out/vfwd-bpf.o
