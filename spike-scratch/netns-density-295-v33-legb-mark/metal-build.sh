#!/bin/bash
# metal host: compile the probe as the login user in /tmp/v33-work only (a real execution target:
# the binary runs on the host and inside the microVM).
set -euo pipefail
W=/tmp/v33-work
export PATH=$HOME/.cargo/bin:$PATH
[ -d $W/out/cargo-home ] || cp -r /tmp/v28-work/out/cargo-home $W/out/cargo-home
export CARGO_HOME=$W/out/cargo-home
RUSTFLAGS="-C target-feature=+crt-static" CARGO_TARGET_DIR=$W/out/target nice -n 19 \
  cargo build --release --offline --target x86_64-unknown-linux-gnu --manifest-path $W/Cargo.toml 2>&1 | tail -20
ls -la $W/out/target/x86_64-unknown-linux-gnu/release/v33
sha256sum $W/out/target/x86_64-unknown-linux-gnu/release/v33
