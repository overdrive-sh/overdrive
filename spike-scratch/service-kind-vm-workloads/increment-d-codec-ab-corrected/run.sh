#!/usr/bin/env bash
set -euo pipefail

readonly INCREMENT=spike-scratch/service-kind-vm-workloads/increment-d-codec-ab-corrected
readonly MANIFEST="$INCREMENT/Cargo.toml"
readonly TARGET=x86_64-unknown-linux-musl
readonly OUT="$INCREMENT/out"

mkdir -p "$OUT"

echo "PREDICTION json_incremental_binary_smaller_or_equal=UNKNOWN rkyv_wire_smaller=EXPECTED decode_speed_rkyv_faster=EXPECTED"
echo "FALSIFICATION no_material_binary_or_validation_advantage_means_codec_choice_remains_design_tradeoff"
echo "SUBSTRATE kernel=$(uname -sr) arch=$(uname -m) virtualization=$(systemd-detect-virt)"
echo "TOOLCHAIN $(rustc -V) | $(cargo -V)"

build_variant() {
  local name="$1"
  shift
  cargo build --manifest-path "$MANIFEST" --locked --target "$TARGET" --profile release-unstripped "$@"
  cp "$INCREMENT/target/$TARGET/release-unstripped/service-vm-codec-ab-spike" "$OUT/$name.unstripped"
  cargo build --manifest-path "$MANIFEST" --locked --target "$TARGET" --release "$@"
  cp "$INCREMENT/target/$TARGET/release/service-vm-codec-ab-spike" "$OUT/$name.stripped"
}

build_variant baseline --no-default-features
build_variant json --no-default-features --features codec-json
build_variant rkyv --no-default-features --features codec-rkyv

echo "ACTUAL_INIT_SOURCE serde_json_encode=crates/overdrive-core/src/vm/beacon.rs:217 serde_json_decode=crates/overdrive-core/src/vm/beacon.rs:335"
cargo build -p overdrive-init --locked --release --target "$TARGET"
readonly INIT_BIN="target/$TARGET/release/overdrive-init"
echo "ACTUAL_INIT stripped_bytes=$(stat -c %s "$INIT_BIN") format=$(file -b "$INIT_BIN")"
CARGO_TARGET_DIR="$INCREMENT/target/actual-init-unstripped" \
  CARGO_PROFILE_RELEASE_STRIP=none \
  cargo build -p overdrive-init --locked --release --target "$TARGET"
readonly INIT_UNSTRIPPED="$INCREMENT/target/actual-init-unstripped/$TARGET/release/overdrive-init"
init_json_symbols="$(nm -C --defined-only "$INIT_UNSTRIPPED" | grep -c 'serde_json::' || true)"
echo "ACTUAL_INIT_LINK_EVIDENCE unstripped_bytes=$(stat -c %s "$INIT_UNSTRIPPED") defined_serde_json_symbols=$init_json_symbols"

echo "BINARY_SIZES_BEGIN"
for name in baseline json rkyv; do
  unstripped=$(stat -c %s "$OUT/$name.unstripped")
  stripped=$(stat -c %s "$OUT/$name.stripped")
  echo "BINARY name=$name unstripped_bytes=$unstripped stripped_bytes=$stripped"
done
echo "BINARY_SIZES_END"

echo "RUNTIME_BEGIN"
"$OUT/baseline.stripped"
"$OUT/json.stripped"
"$OUT/rkyv.stripped"
echo "RUNTIME_END"

echo "DEPENDENCY_COUNTS_BEGIN"
for spec in "baseline:" "json:codec-json" "rkyv:codec-rkyv"; do
  name="${spec%%:*}"
  feature="${spec#*:}"
  if [[ -n "$feature" ]]; then
    count=$(cargo tree --manifest-path "$MANIFEST" --locked --no-default-features --features "$feature" -e normal --prefix none | sort -u | wc -l)
  else
    count=$(cargo tree --manifest-path "$MANIFEST" --locked --no-default-features -e normal --prefix none | sort -u | wc -l)
  fi
  echo "DEPENDENCIES name=$name unique_normal_tree_lines=$count"
done
echo "DEPENDENCY_COUNTS_END"

echo "SOURCE_FACTS total_lines=$(wc -l < "$INCREMENT/src/main.rs") json_cfg_lines=$(sed -n '/mod selected {/,/^}/p' "$INCREMENT/src/main.rs" | wc -l) note=single_source_cfg_sections_share_framing"
echo "CODEC_AB_COMPLETE"
