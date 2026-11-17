#!/usr/bin/env bash
# V-5 driver. Runs on the Metal host. Output is the evidence.
set -uo pipefail
cd "$(dirname "$0")"
echo "uname -r: $(uname -r)"; echo "cpu: $(grep -m1 'model name' /proc/cpuinfo)"
echo "commit: $(git -C ../.. rev-parse --short HEAD 2>/dev/null)"; rustc --version
echo "== unit-test binary of lib (profile.test) =="; cargo nextest run -p nbp-lib --no-capture 2>&1
echo "== unit-test binary of decls =="; cargo nextest run -p nbp-decls --no-capture 2>&1
echo "== bench binary, lto=thin cgu=1, decls referenced via extern crate =="
cargo bench -p nbp-bench --bench benchmarks 2>&1
echo "== bench binary, decls NOT referenced (cfg nbp_unreferenced) =="
RUSTFLAGS="--cfg nbp_unreferenced" cargo bench -p nbp-bench --bench benchmarks --target-dir target/unref 2>&1
echo "== effective profile =="; grep -A3 'profile.bench' Cargo.toml
