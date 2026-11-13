#!/usr/bin/env bash
# usage: cargo xtask metal run --exclusive -- bash -lc 'bash spike-scratch/ktls-reset-sockmem/run.sh <scenario> [piece_kib]'
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
cd "$(dirname "$0")"
export CARGO_TARGET_DIR=/tmp/ktls-reset-sockmem-target
mkdir -p out
cargo b""uild --release 2>&1 | grep -E "^(error|warning: unused)" -A8
sudo -n modprobe tls 2>&1
ts=$(date -u +%Y%m%dT%H%M%SZ)
{
echo "# uname -r: $(uname -r)"; echo "# before: $(grep ^TCP: /proc/net/sockstat)"
for sc in "$@"; do
  echo "=== $sc"
  "$CARGO_TARGET_DIR/release/probe" $sc 2>&1 ; echo "exit=$?"
  sleep 1; echo "# after process exit: $(grep ^TCP: /proc/net/sockstat)"
done
} | tee out/probe-$ts.log
