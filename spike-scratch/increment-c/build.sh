#!/usr/bin/env bash
# PROBE increment-c (P5) — build the Landlock denial probe.
#
# Deliberately REUSES increment-a's artifacts: the unwrapped arm64 `Image`, the
# ext4 rootfs and the host vsock listener all already exist at
# /var/tmp/spike-increment-a. This increment adds exactly one new binary — the
# denial probe — and a run script that wraps the SAME boot in the [D7]
# confinement flags.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A_OUT=/var/tmp/spike-increment-a
OUT=/var/tmp/spike-increment-c

export RUSTUP_HOME=${RUSTUP_HOME:-/home/marcus.guest/.rustup}
export CARGO_HOME=${CARGO_HOME:-/home/marcus.guest/.cargo}
export CARGO_TARGET_DIR="$HERE/target"

echo "=== [build] verifying increment-a artifacts are present (reuse, not rebuild)"
for f in "$A_OUT/Image" "$A_OUT/rootfs.ext4"; do
  [ -f "$f" ] || { echo "!!! missing $f — run spike-scratch/increment-a/build.sh first"; exit 1; }
  ls -la "$f"
done
A_BIN="$(cd "$HERE/.."/increment-a && pwd)/target/aarch64-unknown-linux-musl/release"
[ -x "$A_BIN/host-listener" ] || { echo "!!! missing $A_BIN/host-listener"; exit 1; }
echo "    reusing host listener: $A_BIN/host-listener"

echo
echo "=== [build] cargo build --release (native; this probe runs on the HOST side)"
find "$HERE/probe/src" -name '*.rs' -exec touch {} +
cargo build --release --manifest-path "$HERE/probe/Cargo.toml"
file "$CARGO_TARGET_DIR/release/landlock-denial"

echo
echo "=== [build] sentinel files that MUST exist for the denial to be unambiguous"
# A denial against a non-existent path is ENOENT, not EACCES — it proves
# nothing. Every deny: target below is a real, readable-as-root file.
mkdir -p "$OUT"
echo "this file is outside every landlock rule handed to the VMM" >"$OUT/SENTINEL-OUTSIDE-RULESET"
ls -la "$OUT/SENTINEL-OUTSIDE-RULESET"
echo "=== [build] DONE"
