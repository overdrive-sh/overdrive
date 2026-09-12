#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e10-metal.XXXXXX")"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
{
  echo '# E10 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run http-status-cross-driver"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout 1800s cargo xtask metal run -- bash -lc \
  "SVM_E08_MATRIX_CAPTURE_ROOT=/tmp/svm-e10-matrix $EXAMPLE run http-status-cross-driver" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"
[[ "$run_rc" -eq 0 ]] || { cat "$EVIDENCE_DIR/product-run.out" >&2; exit "$run_rc"; }
awk '/^--- E10 ledger begin ---$/ { keep=1; next } /^--- E10 ledger end ---$/ { keep=0 } keep' \
  "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/http-status-cross-driver.tsv"
[[ "$(($(wc -l <"$EVIDENCE_DIR/http-status-cross-driver.tsv") - 1))" -eq 8 ]] \
  || { echo 'E10 runner: expected exactly eight ledger rows' >&2; exit 1; }
awk -F '\t' '
  NR == 1 { next }
  NF != 20 || ($1 != "exec" && $1 != "vm") \
    || $2 !~ /^(204|302|404|503)$/ || $3 != "0" \
    || $4 != ("alloc-service-" $1 "-http-" $2 "-0") \
    || $6 !~ /^[0-9]+$/ || $9 !~ /^[0-9]+$/ \
    || ($2 == "204" && ($5 != "Running" || $6 != "0" || $7 != "none" \
        || $8 != "Terminated" || $9 != "0" || $10 != "Terminated" \
        || $11 != "operator-stop-running")) \
    || ($2 != "204" && ($5 != "Running" || $6 !~ /^[1-9][0-9]*$/ \
        || $7 != "deploy+prior-failed+probe" || $8 != "Terminated" \
        || $9 != $6 || $10 != "Terminated" \
        || $11 != "recovered-running-operator-stop")) \
    || $12 != ("status-" $2) || $16 != "0" || $17 != "0" \
    || $18 != "0" || $19 != "0" || $20 != "zero-delta" { exit 1 }
  seen[$1 SUBSEP $2]++
  rows++
  END {
    if (rows != 8) exit 1
    for (driver in drivers) for (status in statuses) \
      if (seen[driver SUBSEP status] != 1) exit 1
  }
  BEGIN {
    drivers["exec"] = 1; drivers["vm"] = 1
    statuses["204"] = 1; statuses["302"] = 1
    statuses["404"] = 1; statuses["503"] = 1
  }
' "$EVIDENCE_DIR/http-status-cross-driver.tsv" \
  || { echo 'E10 runner: terminal trajectory or cleanup audit failed' >&2; exit 1; }
grep -Fq 'E10 PASS: 8/8 Exec/VM HTTP status cells with zero failure-body sentinel exposure' \
  "$EVIDENCE_DIR/product-run.out"
