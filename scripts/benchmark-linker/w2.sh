#!/usr/bin/env bash
# Workload 2 — incremental test rebuild after a one-line src change.
# Reuses target-bench-X/ from workload 1 (must already exist + be warm).
# hyperfine `--warmup 2 --runs 5`.
#
# Forces a real recompile + relink per run by touching the bottom of the
# dep tree (overdrive-core/src/lib.rs) — every other crate plus all 62
# test binaries rebuild. Original file is restored on exit via trap.
#
# Run inside Lima, once per variant, AFTER workload 1:
#   for v in default lld mold wild; do
#     cargo xtask lima run --no-sudo -- bash scripts/benchmark-linker/w2.sh $v
#   done
set -euo pipefail

cd "$(dirname "$0")/../.."

variant="${1:?usage: w2.sh <variant>}"
wild_path=$(command -v wild || true)

case "$variant" in
    default) flags="" ;;
    lld)     flags="-C link-arg=-fuse-ld=lld" ;;
    mold)    flags="-C link-arg=-fuse-ld=mold" ;;
    wild)    flags="-C linker=clang -C link-arg=--ld-path=${wild_path}" ;;
    *)       echo "unknown variant: $variant" >&2; exit 2 ;;
esac

target_dir="target-bench-$variant"
if [[ ! -d "$target_dir/debug" ]]; then
    echo "ERROR: $target_dir not found — run scripts/benchmark-linker/w1.sh $variant first" >&2
    exit 2
fi

out_dir="target/bench-linker"
mkdir -p "$out_dir"

touch_target="crates/overdrive-core/src/lib.rs"
backup="$out_dir/w2-$variant.lib.rs.bak"
cp "$touch_target" "$backup"

cleanup() { mv "$backup" "$touch_target" 2>/dev/null || true; }
trap cleanup EXIT

out_md="$out_dir/w2-$variant.md"
out_json="$out_dir/w2-$variant.json"

echo "=== workload 2: $variant ==="
hyperfine \
    --warmup 2 --runs 5 \
    --shell bash \
    --prepare "echo \"// bench touch \$RANDOM\" >> '$touch_target'" \
    --export-markdown "$out_md" \
    --export-json "$out_json" \
    --command-name "$variant" \
    "CARGO_TARGET_DIR='$target_dir' RUSTFLAGS='$flags' cargo nextest run --no-run --workspace --features integration-tests >/dev/null 2>&1"

echo
echo "--- $out_md ---"
cat "$out_md"
