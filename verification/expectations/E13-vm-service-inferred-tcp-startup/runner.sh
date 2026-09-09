#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e13-metal.XXXXXX")"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
{
  echo '# E13 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run zero-probes"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 1800s cargo xtask metal run -- bash -lc \
  "SVM_E08_MATRIX_CAPTURE_ROOT=/tmp/svm-e13-matrix $EXAMPLE run zero-probes" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"
[[ "$run_rc" -eq 0 ]] || { cat "$EVIDENCE_DIR/product-run.out" >&2; exit "$run_rc"; }
awk '/^--- E13 ledger begin ---$/ { keep=1; next } /^--- E13 ledger end ---$/ { keep=0 } keep' \
  "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/zero-probes.tsv"
[[ "$(($(wc -l <"$EVIDENCE_DIR/zero-probes.tsv") - 1))" -eq 2 ]] \
  || { echo 'E13 runner: expected complementary ledger rows' >&2; exit 1; }
grep -Fq 'E13 PASS: inferred TCP success/failure and complementary VM peer traffic are truthful' \
  "$EVIDENCE_DIR/product-run.out"
