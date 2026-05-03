#!/usr/bin/env bash
# Workload 1 — clean full-workspace test build per linker variant.
# Single timed run (no hyperfine multi-run); clean builds are too long to fit
# 1 + N runs in the 30-min Bash cap. Per-variant target dir is left in place
# for workload 2 to reuse as warm cache.
#
# Run inside Lima, once per variant:
#   for v in default lld mold wild; do
#     cargo xtask lima run --no-sudo -- bash scripts/benchmark-linker/w1.sh $v
#   done
set -euo pipefail

cd "$(dirname "$0")/../.."

variant="${1:?usage: w1.sh <variant>}"
wild_path=$(command -v wild || true)

case "$variant" in
    default) flags="" ;;
    lld)     flags="-C link-arg=-fuse-ld=lld" ;;
    mold)    flags="-C link-arg=-fuse-ld=mold" ;;
    wild)    flags="-C linker=clang -C link-arg=--ld-path=${wild_path}" ;;
    *)       echo "unknown variant: $variant" >&2; exit 2 ;;
esac

target_dir="target-bench-$variant"
out_dir="target/bench-linker"
mkdir -p "$out_dir"

echo "=== workload 1: $variant ==="
echo "RUSTFLAGS=$flags"
echo "CARGO_TARGET_DIR=$target_dir"

rm -rf "$target_dir"

start=$(date +%s)
CARGO_TARGET_DIR="$target_dir" \
RUSTFLAGS="$flags" \
    cargo nextest run --no-run --workspace --features integration-tests \
    > "$out_dir/w1-$variant.log" 2>&1
status=$?
elapsed=$(( $(date +%s) - start ))

# Full log is huge; surface only Compiling/Finished/error lines.
grep -E '^(\[.*?\])?\s*(Compiling|Finished|error)' "$out_dir/w1-$variant.log" \
    | tail -10 || true

if [[ $status -eq 0 ]]; then
    echo "$variant: OK ${elapsed}s" | tee -a "$out_dir/w1-results.txt"
else
    echo "$variant: FAILED ${elapsed}s (exit=$status)" | tee -a "$out_dir/w1-results.txt"
fi
