#!/usr/bin/env bash
# Run on metal (native x86_64 KVM host), as root:
#   OVERDRIVE_BPF_NATIVE=1 cargo xtask metal run -- bash spike-scratch/workload-cgroup/run.sh <mode> [N]
# Prereq (once per tree): cargo xtask bpf-build && cargo xtask guest-kit-prepare  (same as the boot-cost spike)
# modes: build | smoke | a N | b N | c N | d N | cleanup
set -euo pipefail
export PATH=$HOME/.cargo/bin:$PATH
here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
mode="${1:-build}"; N="${2:-10}"
T=/tmp/wc-target
GEN="$here/out/gen"
OBJ=$T/objs
mkdir -p "$OBJ" "$here/evidence"
trap 'chown -R "${SUDO_UID:-0}:${SUDO_GID:-0}" "$here" 2>/dev/null || true' EXIT

build() {
  [ -f "$repo/target/bpf/overdrive_guest_bpf.o" ] || { echo "FATAL: run cargo xtask bpf-build first"; exit 3; }
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --manifest-path "$here/bpf/Cargo.toml"
  cp $T/bpf/bpfel-unknown-none/release/wc-bpf $OBJ/wc_listen.o
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-plus \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core --features plus \
    --manifest-path "$here/bpf/Cargo.toml"
  cp $T/bpf-plus/bpfel-unknown-none/release/wc-bpf $OBJ/wc_plus.o
  # the byte-faithful guest-object copy WITHOUT the listen programs (listen-hooks' baseline object, reused by path)
  RUSTFLAGS="-C linker=bpf-linker -C target-cpu=v3" CARGO_TARGET_DIR=$T/bpf-copy \
    cargo +nightly-2026-08-05 build --release --target bpfel-unknown-none -Z build-std=core \
    --manifest-path "$here/../listen-hooks/bpf/Cargo.toml"
  cp $T/bpf-copy/bpfel-unknown-none/release/listen-hooks-bpf $OBJ/guest_copy.o
  # Generated, gitignored copy of the production init (patched at build time; nothing from crates/ is kept in the repo)
  python3 "$here/gen_init.py" "$repo" "$GEN" "$OBJ/wc_listen.o"
  (cd "$repo" && RUSTFLAGS="--cap-lints=warn" CARGO_TARGET_DIR=$T/init \
    cargo build --release --target x86_64-unknown-linux-musl -p overdrive-init --manifest-path "$GEN/Cargo.toml")
  cp $T/init/x86_64-unknown-linux-musl/release/overdrive-init $OBJ/init-wc
  # same generator, built against the guest object that also carries the listen programs (same-object variants)
  python3 "$here/gen_init.py" "$repo" "$GEN-plus" "$OBJ/wc_listen.o" "$OBJ/wc_plus.o"
  (cd "$repo" && RUSTFLAGS="--cap-lints=warn" CARGO_TARGET_DIR=$T/init-plus \
    cargo build --release --target x86_64-unknown-linux-musl -p overdrive-init --manifest-path "$GEN-plus/Cargo.toml")
  cp $T/init-plus/x86_64-unknown-linux-musl/release/overdrive-init $OBJ/init-wcp
  python3 "$here/gen_init.py" "$repo" "$GEN-copy" "$OBJ/wc_listen.o" "$OBJ/guest_copy.o"
  (cd "$repo" && RUSTFLAGS="--cap-lints=warn" CARGO_TARGET_DIR=$T/init-copy \
    cargo build --release --target x86_64-unknown-linux-musl -p overdrive-init --manifest-path "$GEN-copy/Cargo.toml")
  cp $T/init-copy/x86_64-unknown-linux-musl/release/overdrive-init $OBJ/init-wcc
  (cd "$repo" && CARGO_TARGET_DIR=$T/host cargo build --release --target x86_64-unknown-linux-musl -p wcwl --manifest-path "$here/Cargo.toml")
  cp $T/host/x86_64-unknown-linux-musl/release/wcwl $OBJ/wcwl
  (cd "$repo" && CARGO_TARGET_DIR=$T/host cargo build --release -p wcdriver --manifest-path "$here/Cargo.toml")
  ls -l $OBJ
}

substrate() {
  echo "SUBSTRATE uname_r=$(uname -r) machine=$(uname -m) virt=$(systemd-detect-virt || true) date=$(date -u +%FT%TZ)"
  echo "CH $(/usr/local/bin/cloud-hypervisor --version) sha256=$(sha256sum /usr/local/bin/cloud-hypervisor | cut -d' ' -f1)"
  sha256sum $OBJ/wc_listen.o $OBJ/wc_plus.o $OBJ/guest_copy.o $OBJ/init-wc $OBJ/init-wcp $OBJ/init-wcc $OBJ/wcwl
}

drive() { # warmup variants...
  local warm="$1"; shift
  substrate
  cd "$repo"
  WC_WL_BIN=$OBJ/wcwl $T/host/release/wcdriver "$warm" $OBJ/init-wc $OBJ/init-wcp $OBJ/init-wcc "$@"
}

leftovers() {
  echo "LEFTOVERS workload cgroup dirs:"; ls -d /sys/fs/cgroup/workload 2>&1 || true
  echo "LEFTOVERS wc bpf programs:"; bpftool prog show 2>/dev/null | grep -E "wc_(listen|post_bind)" || echo none
  echo "LEFTOVERS cloud-hypervisor procs:"; pgrep -af cloud-hypervisor || echo none
  echo "LEFTOVERS wcdriver procs:"; pgrep -af wcdriver || echo none
}

case "$mode" in
  build) build ;;
  smoke) drive 0 s1=child,single,1 s4=child,corr,2 s5=rootearly,single,1 s6=base,corr,2 s7=rootlate,corr,1 s8=child-nobtf,single,2 s9=cgonly,single,1 s10=outside,single,1 s11=sameobj,single,2,p s12=sameobj,corr,1,p s13=sameobj-idle,single,1,p s14=base,single,1,c ;;
  a) drive 2 a_single=child,single,$((N*4/10)) a_childonly=child,childonly,$((N*3/10)) a_parentchild=child,parentchild,$((N*3/10)) ;;
  b) drive 2 b_out_single=outside,single,$N b_out_childonly=outside,childonly,$N b_out_parentchild=outside,parentchild,$N b_rootearly=rootearly,single,$N b_rootlate=rootlate,single,$N ;;
  c) drive 2 c_child=child,corr,$N c_outside=outside,corr,$N c_root_base=base,corr,$N c_rootlate=rootlate,corr,$N ;;
  c2) drive 2 c2_child=child,corr,$N c2_nocorr=nocorr,corr,$N c2_root_base=base,corr,$N ;;
  d) drive 3 d_base=base,single,$N d_cgonly=cgonly,single,$N d_child=child,single,$N d_childnobtf=child-nobtf,single,$N d_outside=outside,single,$N d_copybase=base,single,$N,c d_sameobj=sameobj,single,$N,p d_sameobjidle=sameobj-idle,single,$N,p ;;
  cleanup) leftovers ;;
  *) echo "unknown mode $mode"; exit 2 ;;
esac
