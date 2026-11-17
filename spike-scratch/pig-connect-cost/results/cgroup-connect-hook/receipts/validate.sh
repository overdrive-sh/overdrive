#!/usr/bin/env bash
# Validates the retained E-4 record. Run as one exclusive Metal command:
#   cargo xtask metal run --exclusive -- bash spike-scratch/pig-connect-cost/results/cgroup-connect-hook/receipts/validate.sh
ROOT="$OVERDRIVE_METAL_SOURCE_ROOT"
R="$ROOT/spike-scratch/pig-connect-cost/results/cgroup-connect-hook/receipts"
cd "$ROOT"
echo "uname -r: $(uname -r)" > "$R/e4-validate.env"
cargo run -q -p overdrive-bench --bin bench-report -- validate "spike-scratch/pig-connect-cost/results/cgroup-connect-hook/afda96dd5b779f7516937a4f/record.jsonl" > "$R/e4-validate.stdout" 2> "$R/e4-validate.stderr"
echo $? > "$R/e4-validate.status"
cat "$R/e4-validate.status"
