#!/usr/bin/env bash
# E09 v2 native-metal evidence runner.  The v2 example owns build,
# preparation, one serve process, all public lifecycle calls, and cleanup.
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads-v2/run-example.sh"
RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e09-v2-metal.XXXXXX")"
readonly RAW_OUTPUT
readonly V2_OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08-v2"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
{
  echo '# E09 v2 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run tcp-truthfulness-100"
  echo "output_root: $V2_OUTPUT_ROOT"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 14400s cargo xtask metal run -- bash -lc \
  "SVM_E09_V2_OUTPUT_ROOT=$V2_OUTPUT_ROOT $EXAMPLE run tcp-truthfulness-100" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" \
  >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"
[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}

awk '/^--- E09 v2 ledger begin ---$/ { keep=1; next }
  /^--- E09 v2 ledger end ---$/ { keep=0 }
  keep' "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/tcp-truthfulness-100.tsv"
[[ "$(($(wc -l <"$EVIDENCE_DIR/tcp-truthfulness-100.tsv") - 1))" -eq 100 ]] \
  || { echo 'E09 v2 runner: expected exactly 100 ledger rows' >&2; exit 1; }
pass_count="$(awk -F'\t' 'NR > 1 && $2 == "pass" { count++ }
  END { print count + 0 }' "$EVIDENCE_DIR/tcp-truthfulness-100.tsv")"
[[ "$pass_count" -eq 100 ]] \
  || { echo "E09 v2 runner: expected 100 pass rows, observed $pass_count" >&2; exit 1; }
[[ "$(awk -F'\t' 'NR > 1 && $2 == "pass" { print $4 "\t" $5 }
  ' "$EVIDENCE_DIR/tcp-truthfulness-100.tsv" | LC_ALL=C sort -u | wc -l | tr -d ' ')" -eq 1 ]] \
  || { echo 'E09 v2 runner: successful rows did not share one control-plane identity' >&2; exit 1; }
grep -Fq 'E09 v2 PASS: 100/100 truthful pairs through one unchanged control-plane PID; no retries or discarded pairs' \
  "$EVIDENCE_DIR/product-run.out"
cat "$EVIDENCE_DIR/product-run.out"
