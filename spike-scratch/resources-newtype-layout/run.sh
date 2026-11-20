#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
uname -r
cargo --version
cargo run --quiet --release
