#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e09-metal.XXXXXX")"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
{
  echo '# E09 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run tcp-truthfulness-100"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 14400s cargo xtask metal run -- bash -lc \
  "SVM_E08_MATRIX_CAPTURE_ROOT=/tmp/svm-e09-matrix $EXAMPLE run tcp-truthfulness-100" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"
[[ "$run_rc" -eq 0 ]] || { cat "$EVIDENCE_DIR/product-run.out" >&2; exit "$run_rc"; }
awk '/^--- E09 ledger begin ---$/ { keep=1; next } /^--- E09 ledger end ---$/ { keep=0 } keep' \
  "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/tcp-truthfulness-100.tsv"
[[ "$(($(wc -l <"$EVIDENCE_DIR/tcp-truthfulness-100.tsv") - 1))" -eq 100 ]] \
  || { echo 'E09 runner: expected exactly 100 ledger rows' >&2; exit 1; }
grep -Fq 'E09 PASS: 100/100 truthful TCP success/failure pairs; no retries or discarded trials' \
  "$EVIDENCE_DIR/product-run.out"
