#!/usr/bin/env bash
# Build ebpf + probe on the metal host (x86_64, nightly-2026-08-05 + bpf-linker). Source dir: /tmp/v16-src
# The artifacts are real execution targets (BPF object + probe binary).
set -euo pipefail
here=/tmp/v16-src
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/tmp/v16-target
mkdir -p /tmp/v16-out
( cd $here/ebpf && rustup run nightly-2026-08-05 cargo build --release --target bpfel-unknown-none -Z build-std=core )
cp /tmp/v16-target/bpfel-unknown-none/release/v16-bpf /tmp/v16-out/v16-bpf.o
( cd $here/probe && cargo build --release )
cp /tmp/v16-target/release/v16 /tmp/v16-out/v16
# follow-up variant: same program, BPF_F_ADJ_ROOM_FIXED_GSO on GSO skbs
( cd $here/ebpf && CARGO_TARGET_DIR=/tmp/v16-target-fgso rustup run nightly-2026-08-05 cargo build --release --features fgso --target bpfel-unknown-none -Z build-std=core )
cp /tmp/v16-target-fgso/bpfel-unknown-none/release/v16-bpf /tmp/v16-out/v16-bpf-fgso.o
ls -l /tmp/v16-out
