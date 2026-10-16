#!/bin/bash
# Build evprobe (static) and stage the module + the V-3/valve control artifacts, in /tmp/evmod-work/out only.
set -euo pipefail
W=/tmp/evmod-work
export PATH=$HOME/.cargo/bin:$PATH
export CARGO_HOME=$W/out/cargo-home-bench
mkdir -p $W/out
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --offline --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml 2>&1 | tail -40
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/evprobe $W/out/evprobe
# the V-3/valve control probe and Cloud Hypervisor fork binary are reused byte-for-byte from the earlier spikes
cp -f /tmp/valve-work/out/valveprobe /tmp/valve-work/out/valve-bpf.o /tmp/valve-work/out/cloud-hypervisor $W/out/
rm -rf $W/out/mod-evmod && mkdir -p $W/out/mod-evmod && cp $W/src/module/* $W/out/mod-evmod/
make -C /lib/modules/$(uname -r)/build M=$W/out/mod-evmod modules 2>&1 | tail -5
sha256sum $W/out/evprobe $W/out/valveprobe $W/out/valve-bpf.o $W/out/cloud-hypervisor $W/out/mod-evmod/evmod.ko
