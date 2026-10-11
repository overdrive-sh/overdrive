#!/bin/bash
# v30: build on the metal host (run via launch.py under the lease), only in /tmp/v30-work.
# stock CH = vendors/cloud-hypervisor as in the repo (fork 41a619d19);
# fd CH    = stock + fd-mode.patch (the V-25 prototype, with its once-only guard).
set -euo pipefail
W=/tmp/v30-work; export PATH=$HOME/.cargo/bin:$PATH
SRCREPO=/home/ubuntu/overdrive; V24=$SRCREPO/spike-scratch/netns-density-295-v24
mkdir -p $W/out; cd $W
[ -d out/cargo-home ] || cp -r $V24/out/cargo-home out/cargo-home
export CARGO_HOME=$W/out/cargo-home
CARGO_TARGET_DIR=$W/out/guest-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/guest/Cargo.toml 2>&1 | tail -20
cp -f out/guest-target/x86_64-unknown-linux-gnu/release/v30guest out/v30guest
rm -rf out/ch-src-stock out/ch-src-fd; mkdir -p out/ch-src-stock out/ch-src-fd
(cd $SRCREPO/vendors/cloud-hypervisor && tar --exclude=.git --exclude=target -cf - .) | tar -xf - -C out/ch-src-stock
cp -a out/ch-src-stock/. out/ch-src-fd/
(cd out/ch-src-fd && patch -p1 < $W/fd-mode.patch)
[ -d out/ch-target ] || cp -r $V24/out/ch-target out/ch-target
export CARGO_TARGET_DIR=$W/out/ch-target
nice -n 19 cargo build --release --locked --manifest-path out/ch-src-stock/Cargo.toml --bin cloud-hypervisor 2>&1 | tail -5
cp -f out/ch-target/release/cloud-hypervisor out/ch-stock
nice -n 19 cargo build --release --locked --manifest-path out/ch-src-fd/Cargo.toml --bin cloud-hypervisor 2>&1 | tail -5
cp -f out/ch-target/release/cloud-hypervisor out/ch-fd
sha256sum out/v30guest out/ch-stock out/ch-fd
(cd out/ch-src-stock && sha256sum virtio-devices/src/vsock/vhost_kernel.rs vmm/src/lib.rs vmm/src/vm_config.rs)
