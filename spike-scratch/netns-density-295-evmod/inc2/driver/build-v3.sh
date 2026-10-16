#!/bin/bash
# Compile-only check of the V-3 host-role harness (bench-v3), in /var/tmp/evmod2-work/out only. The first increment never got this far.
set -uo pipefail
W=/var/tmp/evmod2-work
export PATH=$HOME/.cargo/bin:$PATH
export CARGO_HOME=$W/out/cargo-home-bench
CARGO_TARGET_DIR=$W/out/bench-v3-check cargo build --release --locked --offline --manifest-path $W/src/bench-v3/Cargo.toml 2>&1 | tail -80
echo "build-v3 rc=${PIPESTATUS[0]}"
