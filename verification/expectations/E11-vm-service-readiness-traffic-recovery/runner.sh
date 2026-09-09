#!/usr/bin/env bash
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads/run-example.sh"
readonly REMOTE_RUN_BUDGET_SECONDS=1200
readonly REMOTE_CLEANUP_GRACE_SECONDS=90
readonly TRANSPORT_TIMEOUT_SECONDS=$((REMOTE_RUN_BUDGET_SECONDS + REMOTE_CLEANUP_GRACE_SECONDS + 60))
readonly RAW_OUTPUT="$(mktemp "${TMPDIR:-/tmp}/svm-e11-metal.XXXXXX")"
readonly OUTPUT_ROOT="/srv/vm/overdrive-testing/svm-e08"
export OVERDRIVE_METAL_KERNEL="${OVERDRIVE_METAL_KERNEL:-/srv/vm/overdrive-testing/kernel}"
export OVERDRIVE_METAL_ROOTFS="${OVERDRIVE_METAL_ROOTFS:-/srv/vm/overdrive-testing/rootfs.ext4}"

cleanup() { rm -f -- "$RAW_OUTPUT"; }
trap cleanup EXIT HUP INT TERM

"$REPO_ROOT/$EXAMPLE" check-source
{
  echo '# E11 invocation'
  echo "command: cargo xtask metal run -- $EXAMPLE run readiness-recovery"
  echo "remote_command: timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E08_OUTPUT_ROOT=$OUTPUT_ROOT $EXAMPLE run readiness-recovery"
  echo "remote_timeout: ${REMOTE_RUN_BUDGET_SECONDS}s journey + ${REMOTE_CLEANUP_GRACE_SECONDS}s cleanup grace"
  echo "transport_timeout: ${TRANSPORT_TIMEOUT_SECONDS}s"
  echo "output_root: $OUTPUT_ROOT"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout --foreground --signal=TERM --kill-after=30s "${TRANSPORT_TIMEOUT_SECONDS}s" \
  cargo xtask metal run -- bash -lc \
  "timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E08_OUTPUT_ROOT=$OUTPUT_ROOT $EXAMPLE run readiness-recovery" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" \
  >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

# Extract before checking the remote result so an early timeout retains any
# partial describe output and ledger rows for the independent audit.
awk '/^--- E11 ledger begin ---$/ { keep=1; next }
  /^--- E11 ledger end ---$/ { keep=0 }
  keep' "$EVIDENCE_DIR/product-run.out" >"$EVIDENCE_DIR/readiness-recovery.tsv"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}

row_count="$(awk -F '\t' 'NR > 1 { count++ } END { print count + 0 }' \
  "$EVIDENCE_DIR/readiness-recovery.tsv")"
[[ "$row_count" -eq 3 ]] \
  || { echo "E11 runner: expected exactly three phase rows, observed $row_count" >&2; exit 1; }

awk -F '\t' '
  NR == 1 {
    if ($0 != "phase\treadiness\tobserved_at_ms\tdetected_at_ms\ttransition_latency_ms\tclient_started_at_ms\tclient_elapsed_ms\tlifecycle\trestarts\tpeer_result") exit 1
    next
  }
  NF != 10 || ($1 != "before" && $1 != "during" && $1 != "after") \
    || (($1 == "before" || $1 == "after") && $2 != "pass") \
    || ($1 == "during" && $2 != "fail") \
    || $3 !~ /^[0-9]+$/ || $4 !~ /^[0-9]+$/ || $5 !~ /^[0-9]+$/ \
    || $6 !~ /^[0-9]+$/ || $7 !~ /^[0-9]+$/ || $5 > 2000 \
    || $8 != "Running" || $9 != "0" \
    || ($1 == "before" && $10 != "exact-reply") \
    || ($1 == "during" && $10 != "unreachable-no-exact-reply") \
    || ($1 == "after" && $10 != "exact-reply") { exit 1 }
  seen[$1]++
  rows++
  END {
    if (rows != 3 || seen["before"] != 1 || seen["during"] != 1 || seen["after"] != 1) exit 1
  }
' "$EVIDENCE_DIR/readiness-recovery.tsv" \
  || { echo 'E11 runner: phase, transition-bound, lifecycle, restart, or peer assertions failed' >&2; exit 1; }

[[ "$(grep -Fc 'Verdict: Succeeded' "$EVIDENCE_DIR/product-run.out")" -ge 3 ]] \
  || { echo 'E11 runner: expected successful public verdicts for all three peer Jobs' >&2; exit 1; }
grep -Fq 'E11 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0 mount=0 loop=0 preparation=0' \
  "$EVIDENCE_DIR/product-run.out" \
  || { echo 'E11 runner: teardown did not leave a zero cleanup delta' >&2; exit 1; }
grep -Fq 'E11 PASS: 2/2 readiness transitions within two seconds; exact peer replies before/after and no failed-window reply' \
  "$EVIDENCE_DIR/product-run.out"
cat "$EVIDENCE_DIR/product-run.out"
