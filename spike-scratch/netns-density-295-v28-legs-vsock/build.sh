#!/bin/bash
# v28: build on the metal host as the login user in /tmp/v28-work only (a real execution target,
# not a typecheck loop; the binary runs in a guest and on the host).
set -euo pipefail
W=/tmp/v28-work
export PATH=$HOME/.cargo/bin:$PATH
mkdir -p $W/out
SRC=/home/ubuntu/overdrive/spike-scratch
[ -d $W/out/cargo-home ] || cp -r $SRC/netns-density-295-guest-vsock-capture/out/cargo-home $W/out/cargo-home
export CARGO_HOME=$W/out/cargo-home
cp -f $SRC/netns-density-295-v24/out/ch-target/release/cloud-hypervisor $W/out/cloud-hypervisor
CARGO_TARGET_DIR=$W/out/bpf-target RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" nice -n 19 \
  cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --manifest-path $W/src/bpf/Cargo.toml 2>&1 | tail -20
cp -f $W/out/bpf-target/bpfel-unknown-none/release/tp-bpf $W/src/probe/tp-bpf.o
CARGO_TARGET_DIR=$W/out/probe-target RUSTFLAGS="-C target-feature=+crt-static" nice -n 19 \
  cargo build --release --target x86_64-unknown-linux-gnu --manifest-path $W/src/probe/Cargo.toml 2>&1 | tail -80
cp -f $W/out/probe-target/x86_64-unknown-linux-gnu/release/legs $W/out/legs
sha256sum $W/out/legs $W/out/cloud-hypervisor
# TLS test PKI: CA + leaf (EC P-256, SAN legs.test); server presents the leaf, client trusts the CA
cd $W/out
if [ ! -f cert.pem ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout ca.key -out ca.pem -days 3 -subj /CN=legs-ca -addext basicConstraints=critical,CA:TRUE -addext keyUsage=critical,keyCertSign 2>&1 | tail -1
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes -keyout key.pem -out leaf.csr -subj /CN=legs.test 2>&1 | tail -1
  printf 'subjectAltName=DNS:legs.test\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature\n' > ext.cnf
  openssl x509 -req -in leaf.csr -CA ca.pem -CAkey ca.key -CAcreateserial -out cert.pem -days 3 -extfile ext.cnf 2>&1 | tail -1
fi
ls -la cert.pem ca.pem key.pem
