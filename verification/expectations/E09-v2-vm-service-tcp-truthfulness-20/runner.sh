#!/usr/bin/env bash
# E09 v2 native-metal evidence runner.  The v2 example owns build,
# preparation, one serve process, all public lifecycle calls, and cleanup.
set -euo pipefail

readonly EXAMPLE="examples/service-kind-vm-workloads-v2/run-example.sh"
readonly REMOTE_RUN_BUDGET_SECONDS=600
readonly REMOTE_CLEANUP_GRACE_SECONDS=60
# Leave a bounded bootstrap/transport margin while allowing the complete
# remote run budget and its cleanup grace to elapse.
readonly TRANSPORT_TIMEOUT_SECONDS=$((REMOTE_RUN_BUDGET_SECONDS + REMOTE_CLEANUP_GRACE_SECONDS + 60))
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
  echo "command: cargo xtask metal run -- $EXAMPLE run tcp-truthfulness-20"
  echo "remote_command: timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E09_V2_OUTPUT_ROOT=$V2_OUTPUT_ROOT SVM_E09_V2_CONCURRENCY=10 $EXAMPLE run tcp-truthfulness-20"
  echo "remote_timeout: ${REMOTE_RUN_BUDGET_SECONDS}s setup-and-trials + ${REMOTE_CLEANUP_GRACE_SECONDS}s cleanup grace"
  echo "transport_timeout: ${TRANSPORT_TIMEOUT_SECONDS}s"
  echo "output_root: $V2_OUTPUT_ROOT"
  echo "started_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
} >"$EVIDENCE_DIR/product-run.meta"

set +e
timeout --foreground --signal=TERM --kill-after=15s "${TRANSPORT_TIMEOUT_SECONDS}s" \
  cargo xtask metal run -- bash -lc \
  "timeout --signal=TERM --kill-after=${REMOTE_CLEANUP_GRACE_SECONDS}s ${REMOTE_RUN_BUDGET_SECONDS}s env SVM_E09_V2_OUTPUT_ROOT=$V2_OUTPUT_ROOT SVM_E09_V2_CONCURRENCY=10 $EXAMPLE run tcp-truthfulness-20" \
  >"$RAW_OUTPUT" 2>&1
run_rc=$?
set -e
sed -E 's/@[A-Za-z0-9._-]+/@<metal-host-redacted>/g' <"$RAW_OUTPUT" \
  >"$EVIDENCE_DIR/product-run.out"
{
  echo "finished_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "exit: $run_rc"
} >>"$EVIDENCE_DIR/product-run.meta"

# Extract before checking the process result so a deadline or transport failure
# leaves every partial ledger row in durable local evidence.
LEDGER_EVIDENCE="$EVIDENCE_DIR/tcp-truthfulness-20.tsv"
awk '/^--- E09 v2 ledger begin ---$/ { keep=1; next }
  /^--- E09 v2 ledger end ---$/ { keep=0 }
  keep' "$EVIDENCE_DIR/product-run.out" >"$LEDGER_EVIDENCE"

[[ "$run_rc" -eq 0 ]] || {
  cat "$EVIDENCE_DIR/product-run.out" >&2
  exit "$run_rc"
}

row_count="$(awk -F'\t' 'NR > 1 { count++ } END { print count + 0 }' "$LEDGER_EVIDENCE")"
[[ "$row_count" -eq 20 ]] \
  || { echo "E09 v2 runner: expected exactly 20 ledger rows, observed $row_count" >&2; exit 1; }
pass_count="$(awk -F'\t' 'NR > 1 && $2 == "pass" { count++ }
  END { print count + 0 }' "$LEDGER_EVIDENCE")"
[[ "$pass_count" -eq 20 ]] \
  || { echo "E09 v2 runner: expected 20 pass rows, observed $pass_count" >&2; exit 1; }
[[ "$(awk -F'\t' 'NR > 1 && $2 == "pass" { print $4 "\t" $5 }
  ' "$LEDGER_EVIDENCE" | LC_ALL=C sort -u | wc -l | tr -d ' ')" -eq 1 ]] \
  || { echo 'E09 v2 runner: successful rows did not share one control-plane identity' >&2; exit 1; }
grep -Fq 'E09 v2 PASS: 20/20 truthful' "$EVIDENCE_DIR/product-run.out"
grep -Fq 'no retries' "$EVIDENCE_DIR/product-run.out"
cat "$EVIDENCE_DIR/product-run.out"
