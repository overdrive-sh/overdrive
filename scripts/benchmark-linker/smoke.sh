#!/usr/bin/env bash
# Linker smoke test — confirm each linker produces a working test binary.
# Builds overdrive-core's tests with each variant; per-variant CARGO_TARGET_DIR
# so the linker change is what's measured, not a partial-cache rebuild.
#
# Run inside Lima:
#   cargo xtask lima run --no-sudo -- bash scripts/benchmark-linker/smoke.sh
set -euo pipefail

cd "$(dirname "$0")/../.."

wild_path=$(command -v wild || true)
variants=(default lld mold wild)
declare -A flags=(
    [default]=""
    [lld]="-C link-arg=-fuse-ld=lld"
    [mold]="-C link-arg=-fuse-ld=mold"
    [wild]="-C linker=clang -C link-arg=--ld-path=${wild_path}"
)

out_dir="target/bench-linker"
mkdir -p "$out_dir"
results_file="$out_dir/smoke-results.txt"
: > "$results_file"

for variant in "${variants[@]}"; do
    echo
    echo "=== smoke: $variant ==="
    target_dir="target-bench-$variant"
    rm -rf "$target_dir"

    start=$(date +%s)
    if CARGO_TARGET_DIR="$target_dir" \
       RUSTFLAGS="${flags[$variant]}" \
       cargo nextest run --no-run -p overdrive-core 2>&1 \
       | tail -20; then
        elapsed=$(( $(date +%s) - start ))
        echo "$variant: OK (${elapsed}s)" | tee -a "$results_file"
    else
        elapsed=$(( $(date +%s) - start ))
        echo "$variant: FAILED (${elapsed}s)" | tee -a "$results_file"
    fi
done

echo
echo "=== smoke summary ==="
cat "$results_file"
