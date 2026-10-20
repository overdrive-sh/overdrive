#!/bin/bash
# Build (on the metal host, inside the leased job; nice'd): the module (against the 7.0.0-29 headers), the aya BPF object and the static probe.
# Outputs only in /var/tmp/ev-dgram-work/out. Nothing is loaded.
set -euo pipefail
W=/var/tmp/ev-dgram-work
KVER=7.0.0-29-generic
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
P=/var/tmp/evmod2-work/out
[ -d $W/out/cargo-home ] || cp -r $P/cargo-home-valve $W/out/cargo-home
[ -d $W/out/probe-target ] || cp -r $P/valve-probe-target $W/out/probe-target
[ -d $W/out/bpf-target ] || cp -r $P/valve-bpf-target $W/out/bpf-target
export CARGO_HOME=$W/out/cargo-home
echo "=== probe"
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --locked --offline --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml 2>&1 | tail -30
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/dgprobe $W/out/dgprobe
echo "=== bpf"
set +e
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --offline --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/bpf/Cargo.toml 2>&1 | tail -30
cp -f $W/out/bpf-target/bpfel-unknown-none/release/dg-bpf $W/out/dg-bpf.o || echo BPF_BUILD_FAILED
set -e
echo "=== module"
rm -rf $W/out/mod && mkdir -p $W/out/mod && cp $W/src/module/* $W/out/mod/
make -C /lib/modules/$KVER/build M=$W/out/mod modules 2>&1 | tail -30
echo "=== pins"
sha256sum $W/out/dgprobe $W/out/dg-bpf.o $W/out/mod/evmod.ko $W/src/module/evmod.c || true
modinfo $W/out/mod/evmod.ko | grep -E "vermagic|srcversion|depends"
cp -f /var/tmp/evmod2-work/out/cloud-hypervisor $W/out/cloud-hypervisor
file $W/out/dgprobe | cut -c1-150
