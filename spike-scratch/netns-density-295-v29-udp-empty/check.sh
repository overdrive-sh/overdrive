#!/bin/bash
# V-29: compile the bpf object + vfwd on the metal host as the login user (private dir, nice), same commands as harness.build().
set -euo pipefail
W=/tmp/v29-work
P=/home/ubuntu/overdrive/spike-scratch/netns-density-295-guest-vsock-capture/out
mkdir -p $W/prior
[ -d $W/prior/cargo-home ] || cp -a $P/cargo-home $W/prior/cargo-home
[ -x $W/prior/cloud-hypervisor ] || cp -a $P/ch-target/release/cloud-hypervisor $W/prior/cloud-hypervisor
export CARGO_HOME=$W/prior/cargo-home PATH=$HOME/.cargo/bin:$PATH
cd $W
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --offline --manifest-path $W/bpf/Cargo.toml 2>&1 | tail -${TAILN:-40}
CARGO_TARGET_DIR=$W/out/vfwd-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --offline --manifest-path $W/vfwd/Cargo.toml 2>&1 | tail -${TAILN:-60}
ls -la $W/out/bpf-target/bpfel-unknown-none/release/vfwd-bpf $W/out/vfwd-target/x86_64-unknown-linux-gnu/release/vfwd
