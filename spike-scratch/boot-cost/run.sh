#!/usr/bin/env bash
# Run on metal: OVERDRIVE_BPF_NATIVE=1 cargo xtask metal run -- bash spike-scratch/boot-cost/run.sh [build|run|all] [N] [WARM]
# Prereq (once per tree): cargo xtask bpf-build && cargo xtask guest-kit-prepare
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
mode="${1:-all}"; N="${2:-40}"; WARM="${3:-3}"
T=/tmp/boot-cost-target
out="$here/evidence"; mkdir -p "$out"
trap 'chown -R "${SUDO_UID:-0}:${SUDO_GID:-0}" "$here" 2>/dev/null || true' EXIT
OBJ=$T/objs; mkdir -p $OBJ
cand=$repo/target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o

build() {
  # base = byte-faithful copy of the guest object sources; listen = + merged TCP_LISTEN arm,
  # post_bind4/6, ringbuf + counters (listen-hooks V35 design object, minus its V35-only listen_only prog)
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-base \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --manifest-path "$here/bpf/Cargo.toml"
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-listen \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --features listen --manifest-path "$here/bpf/Cargo.toml"
  cp $T/bpf-base/bpfel-unknown-none/release/boot-cost-bpf $OBJ/spike_base.o
  cp $T/bpf-listen/bpfel-unknown-none/release/boot-cost-bpf $OBJ/spike_listen.o
  cp $repo/target/bpf/overdrive_guest_bpf.o $OBJ/prod.o
  # instrumented copies of overdrive-init (musl static, as vm_fixture::build_overdrive_init_static does)
  mk() { # name obj [extra cargo args]
    BC_GUEST_OBJ=$2 BC_CANDIDATE_OBJ=$cand CARGO_TARGET_DIR=$T/host \
      cargo build --release --target x86_64-unknown-linux-musl -p overdrive-init $3 --manifest-path "$here/Cargo.toml"
    cp $T/host/x86_64-unknown-linux-musl/release/overdrive-init $OBJ/init-$1
  }
  mk A  $OBJ/prod.o ""
  mk A2 $OBJ/spike_base.o ""
  mk B  $OBJ/spike_listen.o "--features listen"
  CARGO_TARGET_DIR=$T/host cargo build --release -p bootcost-driver --manifest-path "$here/Cargo.toml"
  # the REAL unmodified overdrive-init (control R)
  (cd "$repo" && cargo build --package overdrive-init --target x86_64-unknown-linux-musl --release)
  ls -l $OBJ
}

runit() {
  echo "SUBSTRATE uname_r=$(uname -r) machine=$(uname -m) virt=$(systemd-detect-virt || true) date=$(date -u +%FT%TZ)"
  echo "CH $(/usr/local/bin/cloud-hypervisor --version) sha256=$(sha256sum /usr/local/bin/cloud-hypervisor | cut -d' ' -f1)"
  echo "OBJ sha256:"; sha256sum $OBJ/*.o $cand
  ls -l $OBJ/*.o
  cd "$repo"
  tdir=$(cargo metadata --format-version 1 --no-deps 2>/dev/null | python3 -c 'import sys,json;print(json.load(sys.stdin)["target_directory"])')
  REAL=$tdir/x86_64-unknown-linux-musl/release/overdrive-init
  ls -l "$REAL" $OBJ/init-A $OBJ/init-A2 $OBJ/init-B
  $T/host/release/bootcost-driver "$N" "$WARM" R=$REAL A=$OBJ/init-A A2=$OBJ/init-A2 B=$OBJ/init-B
}

case "$mode" in
  driver) CARGO_TARGET_DIR=$T/host cargo build --release -p bootcost-driver --manifest-path "$here/Cargo.toml" ;;
  build) build ;;
  run) runit 2>&1 | tee "$out/run-$(date -u +%Y%m%dT%H%M%SZ).log" ;;
  *) build; runit 2>&1 | tee "$out/run-$(date -u +%Y%m%dT%H%M%SZ).log" ;;
esac
