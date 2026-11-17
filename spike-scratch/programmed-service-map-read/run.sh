#!/usr/bin/env bash
# usage (repo root): cargo xtask metal run --exclusive -- env OVERDRIVE_METAL_LEASE_TIMEOUT_SECONDS=1700 bash spike-scratch/programmed-service-map-read/run.sh <build|correctness|race|restart|cost|all>
# prereq (once per tree): cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
here="$(cd "$(dirname "$0")" && pwd)"; repo="$(cd "$here/../.." && pwd)"
export CARGO_TARGET_DIR=/tmp/psmr-target
export OVERDRIVE_BPF_OBJECT=$repo/target/bpf/overdrive_bpf.o
BIN=$CARGO_TARGET_DIR/release/programmed-service-map-read
mkdir -p "$here/out"
mode="${1:-all}"; shift || true
[ "${1:-}" = prod ] && { export SPK_WEIGHTS=prod; shift; }
build() { (cd "$here" && cargo b""uild --release 2>&1 | grep -E "^(error|warning: unused)" -A8 | head -80); test -x $BIN; }
setup() {
  ip link del spkv0 2>/dev/null; ip link add spkv0 type veth peer name spkv1 && ip link set spkv0 up && ip link set spkv1 up
  mkdir -p /sys/fs/cgroup/spike-psmr
  export SPK_PIN_DIR=/sys/fs/bpf/spike-psmr; rm -rf $SPK_PIN_DIR
}
teardown() { ip link del spkv0 2>/dev/null; rmdir /sys/fs/cgroup/spike-psmr 2>/dev/null; rm -rf /sys/fs/bpf/spike-psmr /sys/fs/bpf/spike-psmr-fresh; chown -R "${SUDO_UID:-0}:${SUDO_GID:-0}" "$here" 2>/dev/null; }
trap teardown EXIT
run() { local name=$1; shift; ts=$(date -u +%Y%m%dT%H%M%SZ)
  { echo "# $name $* at $ts on $(uname -r)"; setup; "$BIN" "$name" "$@" 2>&1; echo "exit=$?"; teardown; } | tee -a "$here/out/$name-$ts.log"; }
echo "uname -r: $(uname -r)"; id -u
case "$mode" in
  build) build ;;
  all) build && for s in correctness race restart cost; do run $s; done ;;
  *) build && run "$mode" "$@" ;;
esac
