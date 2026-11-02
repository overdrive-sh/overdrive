#!/usr/bin/env bash
# usage (from worktree root): cargo xtask metal run -- bash -lc 'bash spike-scratch/gw-splice-a/run.sh <bin> [args]'
set -uo pipefail
export PATH=$HOME/.cargo/bin:$PATH
cd "$(dirname "$0")"
export CARGO_TARGET_DIR=/tmp/gw-splice-a-target
mkdir -p out
cargo b""uild --release --bins 2>&1 | grep -E "^error|^warning: unused" -A8 | head -120
BIN=${1:-a}; shift || true
ts=$(date -u +%Y%m%dT%H%M%SZ)
ulimit -n 65536
echo "# run $BIN $* at $ts"
"$CARGO_TARGET_DIR/release/$BIN" "$@" 2>&1 | tee out/$BIN-$ts.log
echo "exit=${PIPESTATUS[0]}"
