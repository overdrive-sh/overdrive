#!/bin/bash
# Build evprobe (static), valveprobe + valve-bpf.o (the sockmap/pump control, rebuilt from the committed valve sources because the
# previous increment's /tmp artifacts were lost in the host reboot), and both module variants, in /var/tmp/evmod2-work/out only.
# The modules are built against the 7.0.0-29-generic headers (the guest and the private QEMU VMs run -29); the physical host now
# boots 7.0.0-38-generic and nothing is loaded into it.
set -euo pipefail
W=/var/tmp/evmod2-work
KVER=7.0.0-29-generic
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
SRC=/home/ubuntu/overdrive/spike-scratch
[ -d $W/out/cargo-home-bench ] || cp -r $SRC/netns-density-295-aya-vsock-proxy/out/cargo-home $W/out/cargo-home-bench
[ -d $W/out/cargo-home-valve ] || cp -r $SRC/netns-density-295-guest-vsock-capture/out/cargo-home $W/out/cargo-home-valve
export CARGO_HOME=$W/out/cargo-home-bench
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --offline --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml 2>&1 | tail -40
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/evprobe $W/out/evprobe
# valve control probe (rebuilt)
export CARGO_HOME=$W/out/cargo-home-valve
CARGO_TARGET_DIR=$W/out/valve-bpf-target RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --offline --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/valve/bpf/Cargo.toml 2>&1 | tail -20
cp -f $W/out/valve-bpf-target/bpfel-unknown-none/release/valve-bpf $W/out/valve-bpf.o
CARGO_TARGET_DIR=$W/out/valve-probe-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --offline --target x86_64-unknown-linux-gnu --manifest-path $W/src/valve/probe/Cargo.toml 2>&1 | tail -20
cp -f $W/out/valve-probe-target/x86_64-unknown-linux-gnu/release/valveprobe $W/out/valveprobe
# the Cloud Hypervisor fork binary used by the previous increment (same sha256 eadddf1b...)
cp -f $SRC/netns-density-295-v24/out/ch-target/release/cloud-hypervisor $W/out/cloud-hypervisor
for m in module module-ktls module-audit; do
  rm -rf $W/out/mod-$m && mkdir -p $W/out/mod-$m && cp $W/src/$m/* $W/out/mod-$m/
  make -C /lib/modules/$KVER/build M=$W/out/mod-$m modules 2>&1 | tail -5
done
sha256sum $W/out/evprobe $W/out/valveprobe $W/out/valve-bpf.o $W/out/cloud-hypervisor $W/out/mod-module/evmod.ko $W/out/mod-module-ktls/evmod.ko $W/out/mod-module-audit/evmod.ko
sha256sum $W/src/module/evmod.c $W/src/module-ktls/evmod.c
modinfo $W/out/mod-module/evmod.ko | grep -E "vermagic|srcversion"
